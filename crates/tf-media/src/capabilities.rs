//! FFmpeg 能力查询（设计方案 §11 / §15 风险 1）。
//!
//! 用 `ffmpeg -encoders` 判断目标格式是否真的可用（例如缺少 `libmp3lame` 时禁用 MP3 输出）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use tf_core::error::{Result, TfError};
use tf_core::model::AudioFormat;

use crate::locate::FfmpegPaths;

/// ffmpeg 版本与可用编码器。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// `ffmpeg -version` 第一行。
    pub version_line: String,
    /// 构建配置（`configuration:` 行）。
    pub configuration: String,
    /// 可用编码器名称列表。
    pub encoders: Vec<String>,
    /// `-encoders` 的原始文本（排错用）。
    pub raw_encoders: String,
}

impl Capabilities {
    /// 是否包含某个编码器。
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.iter().any(|e| e == name)
    }

    /// 输出该格式所需的编码器。
    pub fn required_encoder(format: AudioFormat) -> Option<&'static str> {
        match format {
            AudioFormat::Flac => Some("flac"),
            AudioFormat::Wav => Some("pcm_s24le"),
            AudioFormat::Aiff => Some("pcm_s24be"),
            AudioFormat::Alac => Some("alac"),
            AudioFormat::Mp3 => Some("libmp3lame"),
            AudioFormat::Aac => Some("aac"),
            AudioFormat::Ogg => Some("libvorbis"),
            AudioFormat::Opus => Some("libopus"),
            _ => None,
        }
    }

    /// 该格式是否可编码。
    pub fn supports(&self, format: AudioFormat) -> bool {
        if !format.can_encode() {
            return false;
        }
        match Self::required_encoder(format) {
            Some(enc) => self.has_encoder(enc),
            None => false,
        }
    }

    /// 可用输出格式列表（转换页下拉的真实可用项）。
    pub fn encodable_formats(&self) -> Vec<AudioFormat> {
        AudioFormat::output_formats()
            .into_iter()
            .filter(|f| self.supports(*f))
            .collect()
    }

    /// 缺失编码器的格式（UI 可以灰显并说明原因）。
    pub fn unsupported_formats(&self) -> Vec<(AudioFormat, &'static str)> {
        let mut out = Vec::new();
        for f in AudioFormat::ALL {
            if !f.can_encode() {
                out.push((f, "FFmpeg 无编码器/不在支持范围"));
                continue;
            }
            if let Some(enc) = Self::required_encoder(f) {
                if !self.has_encoder(enc) {
                    out.push((f, enc));
                }
            }
        }
        out
    }
}

/// 解析 `ffmpeg -encoders` 输出中的编码器名。
///
/// 每行形如：` A....D libmp3lame   libmp3lame MP3 (MPEG audio layer 3)`
/// 前 6 个字符是能力标记（首字符为 V/A/S），之后是编码器名。
pub fn parse_encoders(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim_start();
        if trimmed.len() < 8 || !trimmed.is_char_boundary(6) {
            continue;
        }
        let (flags, rest) = trimmed.split_at(6);
        let mut flag_chars = flags.chars();
        let Some(first) = flag_chars.next() else {
            continue;
        };
        if !matches!(first, 'V' | 'A' | 'S') {
            continue;
        }
        if !flag_chars.all(|c| matches!(c, '.' | 'F' | 'S' | 'X' | 'B' | 'D')) {
            continue;
        }
        if !rest.starts_with(' ') {
            continue;
        }
        let name = rest.trim_start().split_whitespace().next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

/// 解析 `ffmpeg -version` 输出。
pub fn parse_version(output: &str) -> (String, String) {
    let version_line = output.lines().next().unwrap_or_default().trim().to_string();
    let configuration = output
        .lines()
        .find_map(|l| l.trim().strip_prefix("configuration:"))
        .unwrap_or_default()
        .trim()
        .to_string();
    (version_line, configuration)
}

/// 查询 ffmpeg 能力。
pub fn query(paths: &FfmpegPaths) -> Result<Capabilities> {
    let version_out = crate::process::command(&paths.ffmpeg)
        .arg("-version")
        .output()
        .map_err(|e| TfError::Unsupported(format!("无法执行 ffmpeg：{e}")))?;
    if !version_out.status.success() {
        return Err(TfError::Unsupported("ffmpeg -version 执行失败".into()));
    }
    let version_text = String::from_utf8_lossy(&version_out.stdout).into_owned();
    let (version_line, configuration) = parse_version(&version_text);

    let encoders_out = crate::process::command(&paths.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map_err(|e| TfError::Unsupported(format!("无法执行 ffmpeg -encoders：{e}")))?;
    let raw_encoders = String::from_utf8_lossy(&encoders_out.stdout).into_owned();

    Ok(Capabilities {
        version_line,
        configuration,
        encoders: parse_encoders(&raw_encoders),
        raw_encoders,
    })
}

/// 检查 ffmpeg 是否可作为 APE 输入解码（不要求输出）。
pub fn can_decode_ape(paths: &FfmpegPaths) -> bool {
    crate::process::command(&paths.ffmpeg)
        .args(["-hide_banner", "-decoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("ape"))
        .unwrap_or(false)
}

/// 校验 ffmpeg 是否在预期位置（UI 设置页用）。
pub fn looks_like_ffmpeg(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("ffmpeg"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCODERS: &str = r#"Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 .F.... = Frame-level multithreading
 ..S... = Slice-level multithreading
 ...X.. = Codec is experimental
 ....B. = Supports draw_horiz_band
 .....D = Supports direct rendering method 1
 ------
 V....D a64multi             Multicolor charset for Commodore 64
 A....D aac                  AAC (Advanced Audio Coding)
 A....D alac                 ALAC (Apple Lossless Audio Codec)
 A....D flac                 FLAC (Free Lossless Audio Codec)
 A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3)
 A....D libopus              libopus Opus
 A....D libvorbis            libvorbis
 A....D pcm_s16le            PCM signed 16-bit little-endian
 A....D pcm_s24le            PCM signed 24-bit little-endian
 A....D pcm_s24be            PCM signed 24-bit big-endian
 A....D vorbis               Vorbis
"#;

    #[test]
    fn parses_encoder_names() {
        let names = parse_encoders(ENCODERS);
        assert!(names.contains(&"aac".to_string()));
        assert!(names.contains(&"libmp3lame".to_string()));
        assert!(names.contains(&"pcm_s24be".to_string()));
        // 表头不应被当成编码器
        assert!(!names.contains(&"Encoders:".to_string()));
        assert!(!names.contains(&"------".to_string()));
    }

    #[test]
    fn supports_uses_required_encoders() {
        let caps = Capabilities {
            version_line: "ffmpeg version 7.0".into(),
            configuration: String::new(),
            encoders: parse_encoders(ENCODERS),
            raw_encoders: ENCODERS.into(),
        };
        assert!(caps.supports(AudioFormat::Flac));
        assert!(caps.supports(AudioFormat::Mp3));
        assert!(caps.supports(AudioFormat::Opus));
        assert!(!caps.supports(AudioFormat::Ape));
        assert_eq!(
            caps.encodable_formats(),
            vec![
                AudioFormat::Flac,
                AudioFormat::Wav,
                AudioFormat::Aiff,
                AudioFormat::Alac,
                AudioFormat::Mp3,
                AudioFormat::Aac,
                AudioFormat::Ogg,
                AudioFormat::Opus
            ]
        );
        assert!(caps
            .unsupported_formats()
            .iter()
            .any(|(f, _)| *f == AudioFormat::Ape));
    }

    #[test]
    fn missing_encoder_disables_format() {
        let no_lame = ENCODERS.replace("libmp3lame", "shn");
        let caps = Capabilities {
            version_line: String::new(),
            configuration: String::new(),
            encoders: parse_encoders(&no_lame),
            raw_encoders: no_lame,
        };
        assert!(!caps.supports(AudioFormat::Mp3));
        assert!(caps
            .unsupported_formats()
            .iter()
            .any(|(f, enc)| *f == AudioFormat::Mp3 && *enc == "libmp3lame"));
    }

    #[test]
    fn parses_version_header() {
        let text = "ffmpeg version 7.0.2-full_build Copyright (c) 2000-2024\nbuilt with gcc 13\n  configuration: --enable-gpl --enable-libmp3lame\n";
        let (v, cfg) = parse_version(text);
        assert!(v.starts_with("ffmpeg version 7.0.2"));
        assert!(cfg.contains("--enable-libmp3lame"));
    }

    #[test]
    fn ffmpeg_name_check() {
        assert!(looks_like_ffmpeg(Path::new("C:/x/ffmpeg.exe")));
        assert!(!looks_like_ffmpeg(Path::new("C:/x/ffprobe.exe")));
    }
}
