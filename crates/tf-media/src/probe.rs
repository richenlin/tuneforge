//! 媒体探测（设计方案 §7.1）：`ffprobe -show_streams -show_format -print_format json`。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tf_core::error::{Result, TfError};
use tf_core::model::{AudioFormat, MediaInfo};

use crate::locate::FfmpegPaths;

/// ffprobe 原始输出（已解析）。
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// 归一化后的媒体信息。
    pub media: MediaInfo,
    /// 原始 JSON（排错用，写入结构化日志）。
    pub raw: Value,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
    error: Option<FfprobeError>,
}

#[derive(Debug, Deserialize, Default)]
struct FfprobeStream {
    codec_name: Option<String>,
    codec_type: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    bits_per_raw_sample: Option<String>,
    bits_per_sample: Option<u32>,
    sample_fmt: Option<String>,
    duration: Option<String>,
    nb_frames: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeError {
    string: Option<String>,
}

/// 构造探测命令参数（可单测）。
pub fn probe_args(file: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-v".into(),
        "error".into(),
        "-print_format".into(),
        "json".into(),
        "-show_streams".into(),
        "-show_format".into(),
        "-show_error".into(),
        file.to_string_lossy().into_owned(),
    ]
}

/// 探测文件（调用 ffprobe）。
pub fn probe_file(paths: &FfmpegPaths, file: &Path) -> Result<ProbeResult> {
    if !file.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", file.display())));
    }
    let output = crate::process::command(&paths.ffprobe)
        .args(probe_args(file))
        .output()
        .map_err(|e| TfError::Probe(format!("无法执行 ffprobe：{e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TfError::Probe(format!(
            "ffprobe 无输出（{}）：{}",
            file.display(),
            stderr.trim()
        )));
    }

    let raw: Value = serde_json::from_str(&stdout)
        .map_err(|e| TfError::Probe(format!("ffprobe JSON 解析失败：{e}")))?;
    media_info_from_value(file, raw)
}

/// 读取媒体信息（探测的常用入口）。
pub fn read_media_info(paths: &FfmpegPaths, file: &Path) -> Result<MediaInfo> {
    Ok(probe_file(paths, file)?.media)
}

/// 把 ffprobe JSON 转成 [`MediaInfo`]（纯函数，便于单测）。
pub fn media_info_from_value(file: &Path, raw: Value) -> Result<ProbeResult> {
    let parsed: FfprobeOutput = serde_json::from_value(raw.clone())
        .map_err(|e| TfError::Probe(format!("ffprobe 输出结构不符合预期：{e}")))?;

    if let Some(err) = &parsed.error {
        if let Some(text) = &err.string {
            return Err(TfError::Probe(format!("{}：{}", file.display(), text)));
        }
    }

    let stream = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    let Some(stream) = stream else {
        return Err(TfError::Unsupported(format!(
            "文件中没有音频流：{}",
            file.display()
        )));
    };

    let format_meta = parsed.format.unwrap_or_default();
    let container = format_meta.format_name.clone().unwrap_or_default();
    let codec = stream.codec_name.clone().unwrap_or_default();
    let audio_format = AudioFormat::from_probe(&container, &codec);

    let sample_rate = stream
        .sample_rate
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|v| *v > 0);

    let bits_per_sample = stream
        .bits_per_raw_sample
        .as_deref()
        .and_then(|s| s.parse::<u16>().ok())
        .or(stream.bits_per_sample.map(|v| v as u16))
        .filter(|v| *v > 0);

    let duration_secs = stream
        .duration
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| {
            format_meta
                .duration
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok())
        })
        .filter(|v| v.is_finite() && *v > 0.0);

    let frames = stream
        .nb_frames
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            duration_secs
                .zip(sample_rate)
                .map(|(d, sr)| (d * sr as f64).round() as u64)
        });

    let size_bytes = format_meta
        .size
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| std::fs::metadata(file).ok().map(|m| m.len()))
        .unwrap_or(0);

    let is_float = stream
        .sample_fmt
        .as_deref()
        .map(|f| f.starts_with("flt") || f.starts_with("dbl"))
        .unwrap_or(false);

    let media = MediaInfo {
        path: PathBuf::from(file),
        format: audio_format,
        container,
        codec,
        sample_rate,
        bits_per_sample,
        is_float,
        channels: stream.channels.map(|c| c as u16),
        channel_layout: stream.channel_layout.clone(),
        duration_secs,
        frames,
        bit_rate: stream
            .bit_rate
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| {
                format_meta
                    .bit_rate
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
            }),
        size_bytes,
    };

    Ok(ProbeResult { media, raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAC_JSON: &str = r#"{
      "streams": [
        {
          "codec_name": "flac",
          "codec_type": "audio",
          "sample_rate": "96000",
          "channels": 2,
          "channel_layout": "stereo",
          "bits_per_raw_sample": "24",
          "sample_fmt": "s32",
          "duration": "245.760000",
          "bits_per_sample": 0,
          "bit_rate": "2304000"
        }
      ],
      "format": {
        "format_name": "flac",
        "duration": "245.780000",
        "size": "71680000",
        "bit_rate": "2300000"
      }
    }"#;

    #[test]
    fn parses_flac_metadata() {
        let raw: Value = serde_json::from_str(FLAC_JSON).unwrap();
        let result = media_info_from_value(Path::new("song.flac"), raw).unwrap();
        let m = result.media;
        assert_eq!(m.format, Some(AudioFormat::Flac));
        assert_eq!(m.sample_rate, Some(96_000));
        assert_eq!(m.bits_per_sample, Some(24));
        assert_eq!(m.channels, Some(2));
        assert_eq!(m.channel_layout.as_deref(), Some("stereo"));
        assert!((m.duration_secs.unwrap() - 245.76).abs() < 1e-6);
        assert_eq!(m.frames, Some((245.76f64 * 96_000.0).round() as u64));
        assert_eq!(m.size_bytes, 71_680_000);
        assert!(!m.is_float);
        assert_eq!(m.duration_label(), "4:06");
    }

    #[test]
    fn parses_dsd_and_detects_float() {
        let json = r#"{
          "streams": [{
            "codec_name": "dsd_lsbf",
            "codec_type": "audio",
            "sample_rate": "2822400",
            "channels": 2,
            "sample_fmt": "fltp",
            "duration": "180.0"
          }],
          "format": {"format_name": "dsf"}
        }"#;
        let raw: Value = serde_json::from_str(json).unwrap();
        let m = media_info_from_value(Path::new("a.dsf"), raw)
            .unwrap()
            .media;
        assert_eq!(m.format, Some(AudioFormat::Dsf));
        assert!(m.is_float);
        assert_eq!(m.bits_per_sample, None);
        assert_eq!(m.sample_rate, Some(2_822_400));
    }

    #[test]
    fn rejects_files_without_audio_stream() {
        let json = r#"{"streams":[{"codec_name":"h264","codec_type":"video"}],"format":{"format_name":"mov"}}"#;
        let raw: Value = serde_json::from_str(json).unwrap();
        let err = media_info_from_value(Path::new("v.mp4"), raw).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Unsupported);
    }

    #[test]
    fn surfaces_ffprobe_error_block() {
        let json = r#"{"streams":[],"error":{"code":-2,"string":"Invalid data found"}}"#;
        let raw: Value = serde_json::from_str(json).unwrap();
        let err = media_info_from_value(Path::new("bad.mp3"), raw).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Probe);
        assert!(err.to_string().contains("Invalid data found"));
    }

    #[test]
    fn duration_falls_back_to_format_section() {
        let json = r#"{
          "streams":[{"codec_name":"mp3","codec_type":"audio","sample_rate":"44100","channels":2}],
          "format":{"format_name":"mp3","duration":"12.5","bit_rate":"320000"}
        }"#;
        let raw: Value = serde_json::from_str(json).unwrap();
        let m = media_info_from_value(Path::new("a.mp3"), raw)
            .unwrap()
            .media;
        assert_eq!(m.duration_secs, Some(12.5));
        assert_eq!(m.bit_rate, Some(320_000));
        assert_eq!(m.frames, Some(551_250));
        assert_eq!(m.bits_per_sample, None);
    }

    #[test]
    fn probe_args_are_json_and_include_show_error() {
        let args = probe_args(Path::new("a.flac"));
        assert!(args.contains(&"-print_format".to_string()));
        assert!(args.contains(&"json".to_string()));
        assert!(args.contains(&"-show_error".to_string()));
        assert_eq!(args.last().unwrap(), "a.flac");
    }
}
