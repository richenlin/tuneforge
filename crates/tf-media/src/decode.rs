//! 解码为 `f64` 平面 PCM（设计方案 §7.2）。
//!
//! `ffmpeg -i in -f f32le -acodec pcm_f32le -` 输出 32-bit float 交织 PCM，
//! Rust 侧按块转换成 `(channels, frames)` 的 `f64` 平面缓冲。

use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tf_core::error::{Result, TfError};
use tf_core::model::AudioBuffer;

use crate::locate::FfmpegPaths;
use crate::progress::{common_io_args, ProgressAcc, ProgressCallback, ProgressStage, StderrPump};

/// 解码选项。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecodeOptions {
    /// 输出采样率（`None` 表示保持源采样率）。
    pub sample_rate: Option<u32>,
    /// 输出声道数（`None` 表示保持）。
    pub channels: Option<u16>,
    /// 起始时间（秒）。
    pub start_secs: Option<f64>,
    /// 解码时长（秒）。
    pub duration_secs: Option<f64>,
    /// 额外 `-af` 滤镜（例如 DSD 的低通）。
    pub filters: Vec<String>,
    /// 预估总时长（用于进度百分比）。
    pub source_duration_secs: Option<f64>,
}

/// 解码结果。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// 平面 `f64` 缓冲。
    pub buffer: AudioBuffer,
}

impl DecodedAudio {
    /// 采样率。
    pub fn sample_rate(&self) -> u32 {
        self.buffer.sample_rate
    }
}

/// 构造解码参数（可单测）。
pub fn decode_args(file: &Path, opts: &DecodeOptions, channels: usize) -> Vec<String> {
    let mut args = common_io_args();
    if let Some(start) = opts.start_secs {
        args.push("-ss".into());
        args.push(format!("{start:.6}"));
    }
    args.push("-i".into());
    args.push(file.to_string_lossy().into_owned());
    if let Some(duration) = opts.duration_secs {
        args.push("-t".into());
        args.push(format!("{duration:.6}"));
    }
    if !opts.filters.is_empty() {
        args.push("-af".into());
        args.push(opts.filters.join(","));
    }
    if let Some(sr) = opts.sample_rate {
        args.push("-ar".into());
        args.push(sr.to_string());
    }
    args.push("-ac".into());
    args.push(channels.to_string());
    args.push("-f".into());
    args.push("f32le".into());
    args.push("-acodec".into());
    args.push("pcm_f32le".into());
    args.push("-".into());
    args
}

/// 解码文件为内存中的平面 `f64` 缓冲。
pub fn decode_to_buffer(
    paths: &FfmpegPaths,
    file: &Path,
    opts: &DecodeOptions,
    progress: Option<Arc<ProgressCallback<'static>>>,
) -> Result<AudioBuffer> {
    if !file.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", file.display())));
    }
    // 需要先知道声道数才能规划平面缓冲：优先用选项，其次用探测结果。
    let channels = match opts.channels {
        Some(c) if c > 0 => c as usize,
        _ => {
            let info = crate::probe::read_media_info(paths, file)?;
            info.channels.unwrap_or(2) as usize
        }
    };

    let args = decode_args(file, opts, channels);
    tracing::debug!(args = ?args, "开始解码");

    let mut cmd = crate::progress::build_command(&paths.ffmpeg, &args);
    if progress.is_some() {
        cmd.stdout(std::process::Stdio::piped());
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| TfError::Decode(format!("无法启动 ffmpeg：{e}")))?;

    let acc = Arc::new(Mutex::new(ProgressAcc::default()));
    let pump = child.stderr.take().map(|stderr| {
        StderrPump::spawn(
            stderr,
            acc.clone(),
            ProgressStage::Decode,
            opts.source_duration_secs,
            progress,
        )
    });

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| TfError::Internal("ffmpeg stdout 不可用".into()))?;

    let mut channel_data: Vec<Vec<f64>> = vec![Vec::new(); channels];
    let mut raw = vec![0u8; 1 << 20];
    let mut carry: Vec<u8> = Vec::with_capacity(4);

    loop {
        let read = stdout
            .read(&mut raw)
            .map_err(|e| TfError::Decode(format!("读取 PCM 失败：{e}")))?;
        if read == 0 {
            break;
        }
        let mut chunk: &[u8] = &raw[..read];
        if !carry.is_empty() {
            let need = 4 - carry.len();
            let take = need.min(chunk.len());
            carry.extend_from_slice(&chunk[..take]);
            chunk = &chunk[take..];
            if carry.len() == 4 {
                let sample = f32::from_le_bytes([carry[0], carry[1], carry[2], carry[3]]);
                channel_data[0].push(sample as f64);
                carry.clear();
            }
        }
        let usable = chunk.len() - chunk.len() % 4;
        for frame in chunk[..usable].chunks_exact(4 * channels) {
            for (ch, bytes) in frame.chunks_exact(4).enumerate() {
                let sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                channel_data[ch].push(sample as f64);
            }
        }
        if usable < chunk.len() {
            carry.extend_from_slice(&chunk[usable..]);
        }
    }

    let status = child
        .wait()
        .map_err(|e| TfError::Decode(format!("等待 ffmpeg 结束失败：{e}")))?;
    if let Some(p) = pump {
        p.join();
    }

    if !status.success() {
        let text = acc
            .lock()
            .map(|g| g.stderr_text())
            .unwrap_or_else(|e| e.into_inner().stderr_text());
        return Err(TfError::Decode(format!(
            "ffmpeg 解码失败（{}）：{}",
            file.display(),
            text.trim()
        )));
    }

    let frames = channel_data.first().map(|v| v.len()).unwrap_or(0);
    let mut buffer = AudioBuffer::new(opts.sample_rate.unwrap_or(48_000), channels, frames);
    for (ch, data) in channel_data.into_iter().enumerate() {
        if data.len() != frames {
            return Err(TfError::Internal(format!(
                "声道 {ch} 样本数 {} 与 {frames} 不一致",
                data.len()
            )));
        }
        buffer.channel_mut(ch).copy_from_slice(&data);
    }

    if frames == 0 {
        return Err(TfError::Decode(format!(
            "解码结果为空：{}",
            file.display()
        )));
    }

    if let Some(sr) = opts.sample_rate {
        buffer.sample_rate = sr;
    } else {
        // 未指定则回读探测结果，避免把采样率写错
        let info = crate::probe::read_media_info(paths, file)?;
        buffer.sample_rate = info.sample_rate.unwrap_or(buffer.sample_rate);
    }

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_args_keep_source_rate_by_default() {
        let args = decode_args(Path::new("a.flac"), &DecodeOptions::default(), 2);
        assert!(args.contains(&"f32le".to_string()));
        assert!(args.contains(&"pcm_f32le".to_string()));
        assert!(!args.contains(&"-ar".to_string()));
        assert_eq!(args.last().unwrap(), "-");
        let ac = args.iter().position(|a| a == "-ac").unwrap();
        assert_eq!(args[ac + 1], "2");
    }

    #[test]
    fn decode_args_apply_resample_seek_and_filters() {
        let opts = DecodeOptions {
            sample_rate: Some(88_200),
            channels: Some(1),
            start_secs: Some(1.5),
            duration_secs: Some(2.0),
            filters: vec!["lowpass=f=30000".into()],
            source_duration_secs: Some(10.0),
        };
        let args = decode_args(Path::new("a.dsf"), &opts, 1);
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss + 1], "1.500000");
        let t = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[t + 1], "2.000000");
        let af = args.iter().position(|a| a == "-af").unwrap();
        assert_eq!(args[af + 1], "lowpass=f=30000");
        let ar = args.iter().position(|a| a == "-ar").unwrap();
        assert_eq!(args[ar + 1], "88200");
    }

    #[test]
    fn missing_file_fails_fast_with_input_category() {
        let paths = FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            source: crate::LocateSource::Path,
        };
        let err = decode_to_buffer(
            &paths,
            Path::new("C:/definitely/not/here.flac"),
            &DecodeOptions::default(),
            None,
        )
        .unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }
}
