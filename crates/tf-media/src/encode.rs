//! 编码（设计方案 §6.2 推荐默认参数 + §7.7）。
//!
//! * 输入统一为 `f32le` 交织 PCM（来自 Rust 侧 DSP，或直接转码）。
//! * 输出由调用方给出（`tf-jobs` 提供临时文件路径，成功后原子 rename）。
//! * 标签不由 ffmpeg 写入（`-map_metadata -1`），由 `tf-tags`/`lofty` 在音频写完后覆盖，
//!   这样标签失败不会破坏音频（§8）。

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tf_core::error::{Result, TfError};
use tf_core::model::{AudioFormat, AudioBuffer, MediaInfo};

use crate::locate::FfmpegPaths;
use crate::capabilities::Capabilities;
use crate::progress::{common_io_args, ProgressAcc, ProgressCallback, ProgressStage, StderrPump};

/// 编码质量参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EncodeQuality {
    /// 无损默认（FLAC 用压缩级别、ALAC/PCM 无参数）。
    Lossless,
    /// FLAC 压缩级别 0–12。
    FlacLevel {
        /// 压缩级别。
        level: u8,
    },
    /// MP3 VBR（0 = V0 最好 ≈ 245 kbps）。
    Mp3Vbr {
        /// VBR 质量 0–9。
        q: u8,
    },
    /// OGG Vorbis 质量 0–10。
    OggQ {
        /// 质量值。
        q: f64,
    },
    /// Opus 固定码率（kbps）。
    OpusBitrate {
        /// 码率 kbps。
        kbps: u32,
    },
    /// AAC VBR（0.1–2，1.5 ≈ 256 kbps）。
    AacVbr {
        /// 质量值。
        q: f64,
    },
    /// AAC 固定码率（kbps）。
    AacBitrate {
        /// 码率 kbps。
        kbps: u32,
    },
    /// PCM（WAV/AIFF）直通，位深由 `bit_depth` 决定。
    Pcm,
}

impl EncodeQuality {
    /// 中文摘要（UI 展示 + 日志）。
    pub fn summary(&self) -> String {
        match self {
            EncodeQuality::Lossless => "无损默认".into(),
            EncodeQuality::FlacLevel { level } => format!("FLAC 压缩级别 {level}"),
            EncodeQuality::Mp3Vbr { q } => format!("MP3 VBR V{q}"),
            EncodeQuality::OggQ { q } => format!("Vorbis q{q:.1}"),
            EncodeQuality::OpusBitrate { kbps } => format!("Opus {kbps} kbps"),
            EncodeQuality::AacVbr { q } => format!("AAC VBR q{q:.1}"),
            EncodeQuality::AacBitrate { kbps } => format!("AAC {kbps} kbps"),
            EncodeQuality::Pcm => "PCM".into(),
        }
    }
}

/// 一次编码的完整参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeSpec {
    /// 目标格式。
    pub format: AudioFormat,
    /// 目标位深；`None` 表示保持源位深。
    pub bit_depth: Option<u16>,
    /// 目标采样率；`None` 表示保持源采样率。
    pub sample_rate: Option<u32>,
    /// 目标声道数；`None` 表示保持。
    pub channels: Option<u16>,
    /// 质量参数。
    pub quality: EncodeQuality,
}

impl EncodeSpec {
    /// 目标文件扩展名（决定“跳过已存在”的判定）。
    pub fn extension(&self) -> &'static str {
        self.format.extension()
    }
}

/// 按设计方案 §6.2 给出推荐默认参数。
pub fn default_spec(format: AudioFormat, source: &MediaInfo) -> Result<EncodeSpec> {
    if !format.can_encode() {
        return Err(TfError::Unsupported(format!(
            "{} 不能作为输出格式（FFmpeg 无对应编码器）",
            format.label()
        )));
    }
    let source_bits = source.bits_per_sample;
    let (bit_depth, quality) = match format {
        // 设计 §15 风险 7：位深默认“保持源”，仅在降位深时才抖动/量化
        AudioFormat::Flac => (
            Some(lossless_bit_depth(source_bits)),
            EncodeQuality::FlacLevel { level: 8 },
        ),
        AudioFormat::Wav | AudioFormat::Aiff => {
            // 源为 16-bit 则 16-bit，否则 24-bit
            let bits = if source_bits == Some(16) { 16 } else { 24 };
            (Some(bits), EncodeQuality::Pcm)
        }
        AudioFormat::Alac => (Some(lossless_bit_depth(source_bits)), EncodeQuality::Lossless),
        AudioFormat::Mp3 => (None, EncodeQuality::Mp3Vbr { q: 0 }),
        AudioFormat::Aac => (None, EncodeQuality::AacVbr { q: 1.5 }),
        AudioFormat::Ogg => (None, EncodeQuality::OggQ { q: 6.0 }),
        AudioFormat::Opus => (None, EncodeQuality::OpusBitrate { kbps: 160 }),
        other => return Err(TfError::Unsupported(format!("暂不支持输出 {}", other.label()))),
    };

    Ok(EncodeSpec {
        format,
        bit_depth,
        sample_rate: None,
        channels: None,
        quality,
    })
}

// ------------------------------------------------------------------ 采样率推荐

/// 目标格式支持的采样率上限（Hz）。
///
/// 超过上限必须重采样；Opus 是硬约束（内部固定 48 kHz）。
pub fn sample_rate_limit(format: AudioFormat) -> u32 {
    match format {
        AudioFormat::Opus => 48_000,
        AudioFormat::Mp3 => 48_000,
        AudioFormat::Aac => 96_000,
        AudioFormat::Ogg => 192_000,
        AudioFormat::Flac => 655_350,
        AudioFormat::Alac | AudioFormat::Wav | AudioFormat::Aiff => 384_000,
        _ => 48_000,
    }
}

/// 目标格式的常用采样率候选（升序）——UI 下拉展示的就是这份列表。
pub fn sample_rate_candidates(format: AudioFormat) -> Vec<u32> {
    match format {
        AudioFormat::Opus => vec![48_000],
        AudioFormat::Mp3 => vec![32_000, 44_100, 48_000],
        AudioFormat::Aac => vec![32_000, 44_100, 48_000, 88_200, 96_000],
        AudioFormat::Ogg => vec![44_100, 48_000, 88_200, 96_000, 192_000],
        AudioFormat::Flac | AudioFormat::Alac | AudioFormat::Wav | AudioFormat::Aiff => {
            vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 384_000]
        }
        _ => Vec::new(),
    }
}

/// 该格式是否直接支持给定采样率（不需要重采样）。
///
/// Opus 特殊：编码器内部固定 48 kHz，只有 48 kHz 是“直接支持”。
pub fn is_sample_rate_supported(format: AudioFormat, rate: u32) -> bool {
    if !format.can_encode() {
        return false;
    }
    if format == AudioFormat::Opus {
        return rate == 48_000;
    }
    rate >= 8_000 && rate <= sample_rate_limit(format)
}

/// 候选里的首选值（需要重采样但源采样率不可用时的回退）。
fn preferred_sample_rate(format: AudioFormat) -> u32 {
    match format {
        AudioFormat::Mp3 | AudioFormat::Ogg => 44_100,
        _ => 48_000,
    }
}

/// 推荐采样率。
///
/// * `None` —— 建议**保持源采样率**（目标格式直接支持，不做任何重采样）；
/// * `Some(rate)` —— 建议重采样到该值（源采样率超出格式上限，或 Opus 这类硬性 48 kHz）。
///
/// 需要重采样时取“不高于源采样率的最大候选值”，尽量保留原始信息量。
pub fn recommended_sample_rate(format: AudioFormat, source: Option<u32>) -> Option<u32> {
    if !format.can_encode() {
        return None;
    }
    match source {
        Some(rate) if is_sample_rate_supported(format, rate) => None,
        Some(rate) => {
            let best = sample_rate_candidates(format)
                .into_iter()
                .filter(|candidate| *candidate <= rate)
                .max();
            Some(best.unwrap_or_else(|| preferred_sample_rate(format)))
        }
        // 源采样率未知：除 Opus 外都不强行重采样
        None => match format {
            AudioFormat::Opus => Some(48_000),
            _ => None,
        },
    }
}

/// 采样率文案（千分位分组，中文界面更好读）。
pub fn format_rate(rate: u32) -> String {
    let text = rate.to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// 每种采样率的用途说明。
fn rate_hint(format: AudioFormat, rate: u32) -> &'static str {
    match (format, rate) {
        (AudioFormat::Opus, 48_000) => "Opus 原生，唯一硬性支持",
        (_, 32_000) => "省空间",
        (_, 44_100) => "CD 标准",
        (_, 48_000) => "视频 / 广播标准",
        (_, 88_200) => "2× CD（母带处理）",
        (_, 96_000) => "高解析",
        (_, 176_400) => "4× CD（母带）",
        (_, 192_000) => "高解析（顶级）",
        (_, 384_000) => "超高解析（DXD）",
        _ => "",
    }
}

/// 采样率下拉项（`value = None` 表示保持源采样率）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleRateOption {
    /// 采样率；`None` = 保持源。
    pub value: Option<u32>,
    /// 中文标签。
    pub label: String,
    /// 是否为推荐项（整份列表只有一项为 `true`）。
    pub recommended: bool,
    /// 保持源时，源采样率是否为目标格式所支持（`value` 为 `None` 时才有意义）。
    pub supported: bool,
}

/// 生成目标格式的采样率下拉选项（第一项总是“保持源”）。
pub fn sample_rate_options(format: AudioFormat, source: Option<u32>) -> Vec<SampleRateOption> {
    let recommended = recommended_sample_rate(format, source);
    let keep_supported = match source {
        Some(rate) => is_sample_rate_supported(format, rate),
        None => true,
    };
    let keep_label = match source {
        Some(rate) if !keep_supported => format!(
            "保持源采样率（{} Hz · 当前格式不支持，将被重采样）",
            format_rate(rate)
        ),
        Some(rate) => format!("保持源采样率（{} Hz）", format_rate(rate)),
        None => "保持源采样率（各文件原样）".to_string(),
    };

    let mut options = vec![SampleRateOption {
        value: None,
        label: keep_label,
        recommended: recommended.is_none(),
        supported: keep_supported,
    }];

    for rate in sample_rate_candidates(format) {
        let hint = rate_hint(format, rate);
        let label = if hint.is_empty() {
            format!("{} Hz", format_rate(rate))
        } else {
            format!("{} Hz · {hint}", format_rate(rate))
        };
        options.push(SampleRateOption {
            value: Some(rate),
            label,
            recommended: recommended == Some(rate),
            supported: true,
        });
    }
    options
}

/// 无损目标的默认位深：能保持源位深就保持（16 / 24），否则 24。
///
/// 避免 16-bit 源被无谓升到 24-bit（体积膨胀），也避免默认降位深（需抖动）。
fn lossless_bit_depth(source_bits: Option<u16>) -> u16 {
    match source_bits {
        Some(bits) if bits <= 24 => bits,
        _ => 24,
    }
}

// ------------------------------------------------------------------ 重采样质量

/// 该 FFmpeg 构建是否带 libsoxr（`-buildconf` 里的 `--enable-libsoxr`）。
///
/// 随包分发的 LGPL 构建带 soxr；若用户自行指定了不带 soxr 的 ffmpeg，
/// 则自动回退到 ffmpeg 默认 resampler，不会因为滤镜不存在而失败。
pub fn has_soxr(caps: &Capabilities) -> bool {
    caps.configuration.contains("--enable-libsoxr")
}

/// 需要改变采样率时的参数。
///
/// 可用 soxr 时用 `aresample=resampler=soxr`（高质量、专业级重采样），
/// 否则回退到 `-ar`（swresample 默认）。两种都会把声道数/位深交给已存在的参数控制。
pub fn resample_args(spec: &EncodeSpec, soxr: bool) -> Vec<String> {
    match spec.sample_rate {
        None => Vec::new(),
        Some(rate) if soxr => vec![
            "-af".into(),
            format!("aresample=osr={rate}:resampler=soxr"),
        ],
        Some(rate) => vec!["-ar".into(), rate.to_string()],
    }
}

// ------------------------------------------------------------------ 转码参数

/// 目标参数与源完全一致时，可以只重封装（`-c:a copy`）：音频比特精确、零重编码损失。
///
/// 判定条件（全部满足）：
/// * 目标格式可编码，且与源格式相同（同一容器）；
/// * `bit_depth` / `sample_rate` / `channels` 为 `None`（保持源）或与源完全相等。
///
/// 也就是说：FLAC→FLAC同参数、MP3→MP3同参数这类“只想换个目录/改标签”的需求
/// 不再过一次编解码，避免有损多次编码与无谓的无损重编。
pub fn can_stream_copy(source: &MediaInfo, spec: &EncodeSpec) -> bool {
    if !spec.format.can_encode() || source.format != Some(spec.format) {
        return false;
    }
    if let Some(bits) = spec.bit_depth {
        if source.bits_per_sample != Some(bits) {
            return false;
        }
    }
    if let Some(rate) = spec.sample_rate {
        if source.sample_rate != Some(rate) {
            return false;
        }
    }
    if let Some(channels) = spec.channels {
        if source.channels != Some(channels) {
            return false;
        }
    }
    true
}

/// 只重封装音频流的参数（供单测）。
pub fn stream_copy_args(input: &Path, out: &Path) -> Vec<String> {
    let mut args = common_io_args();
    args.push("-i".into());
    args.push(input.to_string_lossy().into_owned());
    args.push("-map".into());
    args.push("0:a:0".into());
    args.push("-c:a".into());
    args.push("copy".into());
    args.push("-vn".into());
    args.push("-sn".into());
    args.push("-dn".into());
    args.push("-map_metadata".into());
    args.push("-1".into());
    args.push("-y".into());
    args.push(out.to_string_lossy().into_owned());
    args
}

/// 只重封装音频流（不重编码）。
pub fn transcode_copy(
    paths: &FfmpegPaths,
    input: &Path,
    out: &Path,
    duration_secs: Option<f64>,
    progress: Option<Arc<ProgressCallback<'static>>>,
) -> Result<()> {
    if !input.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", input.display())));
    }
    let args = stream_copy_args(input, out);
    run_encode(paths, &args, out, duration_secs, progress, None)
}

fn pcm_codec(bits: u16, big_endian: bool) -> Result<&'static str> {
    Ok(match (bits, big_endian) {
        (8, _) => "pcm_u8",
        (16, false) => "pcm_s16le",
        (24, false) => "pcm_s24le",
        (32, false) => "pcm_s32le",
        (16, true) => "pcm_s16be",
        (24, true) => "pcm_s24be",
        (32, true) => "pcm_s32be",
        (b, _) => {
            return Err(TfError::Unsupported(format!(
                "PCM 不支持 {b} 位（支持 8/16/24/32）"
            )))
        }
    })
}

/// 编码器参数（可单测）。`soxr = true` 时需改采样率会用 soxr 高质量重采样。
pub fn codec_args_with(spec: &EncodeSpec, soxr: bool) -> Result<Vec<String>> {
    let bits = spec.bit_depth;
    let args: Vec<String> = match spec.format {
        AudioFormat::Flac => {
            let level = match spec.quality {
                EncodeQuality::FlacLevel { level } => level.min(12),
                _ => 8,
            };
            let depth = bits.unwrap_or(24);
            let sample_fmt = if depth <= 16 { "s16" } else { "s32" };
            vec![
                "-c:a".into(),
                "flac".into(),
                "-compression_level".into(),
                level.to_string(),
                "-sample_fmt".into(),
                sample_fmt.into(),
                "-bits_per_raw_sample".into(),
                depth.to_string(),
            ]
        }
        AudioFormat::Wav => {
            let codec = pcm_codec(bits.unwrap_or(24), false)?;
            vec!["-c:a".into(), codec.into()]
        }
        AudioFormat::Aiff => {
            let codec = pcm_codec(bits.unwrap_or(24), true)?;
            vec!["-c:a".into(), codec.into()]
        }
        AudioFormat::Alac => {
            let depth = bits.unwrap_or(24);
            let sample_fmt = if depth <= 16 { "s16p" } else { "s32p" };
            vec![
                "-c:a".into(),
                "alac".into(),
                "-sample_fmt".into(),
                sample_fmt.into(),
            ]
        }
        AudioFormat::Mp3 => {
            let q = match spec.quality {
                EncodeQuality::Mp3Vbr { q } => q.min(9),
                _ => 0,
            };
            vec![
                "-c:a".into(),
                "libmp3lame".into(),
                "-q:a".into(),
                q.to_string(),
            ]
        }
        AudioFormat::Aac => match spec.quality {
            EncodeQuality::AacBitrate { kbps } => vec![
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{kbps}k"),
            ],
            EncodeQuality::AacVbr { q } => vec![
                "-c:a".into(),
                "aac".into(),
                "-q:a".into(),
                format!("{:.2}", q.clamp(0.1, 2.0)),
            ],
            _ => vec!["-c:a".into(), "aac".into(), "-q:a".into(), "1.50".into()],
        },
        AudioFormat::Ogg => {
            let q = match spec.quality {
                EncodeQuality::OggQ { q } => q.clamp(0.0, 10.0),
                _ => 6.0,
            };
            vec![
                "-c:a".into(),
                "libvorbis".into(),
                "-q:a".into(),
                format!("{q:.1}"),
            ]
        }
        AudioFormat::Opus => {
            let kbps = match spec.quality {
                EncodeQuality::OpusBitrate { kbps } => kbps,
                _ => 160,
            };
            vec![
                "-c:a".into(),
                "libopus".into(),
                "-b:a".into(),
                format!("{kbps}k"),
            ]
        }
        other => {
            return Err(TfError::Unsupported(format!(
                "{} 不能作为输出格式",
                other.label()
            )))
        }
    };

    // 通用重采样 / 声道参数
    let mut out = args;
    out.extend(resample_args(spec, soxr));
    if let Some(ch) = spec.channels {
        out.push("-ac".into());
        out.push(ch.to_string());
    }
    Ok(out)
}

/// 兼容入口：不启用 soxr（非改采样率场景结果一致）。
pub fn codec_args(spec: &EncodeSpec) -> Result<Vec<String>> {
    codec_args_with(spec, false)
}

/// 由 PCM 缓冲编码（Rust DSP 之后的路径）。
pub fn encode_buffer(
    paths: &FfmpegPaths,
    spec: &EncodeSpec,
    buffer: &AudioBuffer,
    out: &Path,
    duration_secs: Option<f64>,
    soxr: bool,
    progress: Option<Arc<ProgressCallback<'static>>>,
) -> Result<()> {
    if !spec.format.can_encode() {
        return Err(TfError::Unsupported(format!(
            "{} 不能作为输出格式",
            spec.format.label()
        )));
    }
    if buffer.frames == 0 {
        return Err(TfError::Encode("输入 PCM 为空".into()));
    }

    let mut args = common_io_args();
    args.push("-f".into());
    args.push("f32le".into());
    args.push("-ar".into());
    args.push(buffer.sample_rate.to_string());
    args.push("-ac".into());
    args.push(buffer.channels.to_string());
    args.push("-i".into());
    args.push("-".into());
    args.push("-vn".into());
    args.push("-map_metadata".into());
    args.push("-1".into());
    args.extend(codec_args_with(spec, soxr)?);
    args.push("-y".into());
    args.push(out.to_string_lossy().into_owned());

    run_encode(
        paths,
        &args,
        out,
        duration_secs,
        progress,
        Some(PcmWriter::new(buffer)),
    )
}

/// 直接转码（ffmpeg → ffmpeg，不经过 Rust DSP；用于纯格式转换）。
pub fn transcode(
    paths: &FfmpegPaths,
    spec: &EncodeSpec,
    input: &Path,
    out: &Path,
    duration_secs: Option<f64>,
    keep_metadata: bool,
    soxr: bool,
    progress: Option<Arc<ProgressCallback<'static>>>,
) -> Result<()> {
    if !input.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", input.display())));
    }
    let mut args = common_io_args();
    args.push("-i".into());
    args.push(input.to_string_lossy().into_owned());
    args.push("-map".into());
    args.push("0:a:0".into());
    args.push("-vn".into());
    args.push("-sn".into());
    args.push("-dn".into());
    args.push("-map_metadata".into());
    args.push(if keep_metadata { "0" } else { "-1" }.into());
    args.extend(codec_args_with(spec, soxr)?);
    args.push("-y".into());
    args.push(out.to_string_lossy().into_owned());

    match run_encode(paths, &args, out, duration_secs, progress, None) {
        Ok(()) => Ok(()),
        Err(err) => Err(err),
    }
}

/// PCM 写入器：把平面 `f64` 缓冲分块转成 `f32le` 字节写进 stdin。
struct PcmWriter<'a> {
    buffer: &'a AudioBuffer,
    frames_written: usize,
}

impl<'a> PcmWriter<'a> {
    fn new(buffer: &'a AudioBuffer) -> Self {
        PcmWriter {
            buffer,
            frames_written: 0,
        }
    }

    fn write_to(&mut self, sink: &mut dyn Write) -> std::io::Result<()> {
        const CHUNK_FRAMES: usize = 32_768;
        let channels = self.buffer.channels;
        let mut bytes = Vec::with_capacity(CHUNK_FRAMES * channels * 4);
        while self.frames_written < self.buffer.frames {
            let end = (self.frames_written + CHUNK_FRAMES).min(self.buffer.frames);
            bytes.clear();
            for i in self.frames_written..end {
                for ch in 0..channels {
                    let s = self.buffer.data[ch * self.buffer.frames + i] as f32;
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
            }
            sink.write_all(&bytes)?;
            self.frames_written = end;
        }
        Ok(())
    }
}

/// 运行编码子进程。
fn run_encode(
    paths: &FfmpegPaths,
    args: &[String],
    out: &Path,
    duration_secs: Option<f64>,
    progress: Option<Arc<ProgressCallback<'static>>>,
    mut pcm: Option<PcmWriter<'_>>,
) -> Result<()> {
    tracing::debug!(args = ?args, "开始编码");
    let mut cmd = crate::process::command(&paths.ffmpeg);
    cmd.args(args)
        .stdin(if pcm.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| TfError::Encode(format!("无法启动 ffmpeg：{e}")))?;

    let acc = Arc::new(Mutex::new(ProgressAcc::default()));
    let pump = child.stderr.take().map(|stderr| {
        StderrPump::spawn(
            stderr,
            acc.clone(),
            ProgressStage::Encode,
            duration_secs,
            progress,
        )
    });

    if let Some(writer) = pcm.as_mut() {
        if let Some(mut stdin) = child.stdin.take() {
            let write_result = writer.write_to(&mut stdin);
            drop(stdin);
            if let Err(e) = write_result {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(p) = pump {
                    p.join();
                }
                return Err(TfError::Encode(format!("写入 PCM 失败：{e}")));
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| TfError::Encode(format!("等待 ffmpeg 结束失败：{e}")))?;
    if let Some(p) = pump {
        p.join();
    }

    let stderr_text = acc
        .lock()
        .map(|g| g.stderr_text())
        .unwrap_or_else(|e| e.into_inner().stderr_text());

    if !status.success() {
        return Err(TfError::Encode(format!(
            "ffmpeg 编码失败（状态 {status}）：{}",
            stderr_text.trim()
        )));
    }

    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(TfError::Encode(format!(
            "编码输出为空：{} {}",
            out.display(),
            stderr_text.trim()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---------------------------------------------------------- 采样率推荐

    #[test]
    fn candidates_respect_format_support() {
        for format in [
            AudioFormat::Flac,
            AudioFormat::Wav,
            AudioFormat::Aiff,
            AudioFormat::Alac,
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Ogg,
            AudioFormat::Opus,
        ] {
            let candidates = sample_rate_candidates(format);
            assert!(!candidates.is_empty(), "{format:?} 应有候选值");
            for &rate in &candidates {
                assert!(is_sample_rate_supported(format, rate), "{format:?} 候选 {rate} 应被支持");
            }
            assert!(candidates.windows(2).all(|w| w[0] < w[1]), "候选必须升序且无重复");
        }
        // 关键硬约束
        assert_eq!(sample_rate_candidates(AudioFormat::Opus), vec![48_000]);
        assert_eq!(sample_rate_candidates(AudioFormat::Mp3), vec![32_000, 44_100, 48_000]);
        assert!(sample_rate_candidates(AudioFormat::Ape).is_empty(), "不可编码格式没有候选");
        assert!(!is_sample_rate_supported(AudioFormat::Mp3, 96_000));
        assert!(!is_sample_rate_supported(AudioFormat::Opus, 44_100));
        assert!(!is_sample_rate_supported(AudioFormat::Ape, 44_100));
        assert!(is_sample_rate_supported(AudioFormat::Flac, 192_000));
    }

    #[test]
    fn recommendation_keeps_source_when_supported() {
        assert_eq!(recommended_sample_rate(AudioFormat::Flac, Some(44_100)), None);
        assert_eq!(recommended_sample_rate(AudioFormat::Mp3, Some(44_100)), None);
        assert_eq!(recommended_sample_rate(AudioFormat::Mp3, Some(8_000)), None);
        assert_eq!(recommended_sample_rate(AudioFormat::Aac, Some(96_000)), None);
        assert_eq!(recommended_sample_rate(AudioFormat::Opus, Some(48_000)), None);
        assert_eq!(recommended_sample_rate(AudioFormat::Flac, None), None);
    }

    #[test]
    fn recommendation_resamples_only_when_required() {
        // 96 kHz 源 → MP3 上限 48 kHz，取不高于源的最大候选
        assert_eq!(recommended_sample_rate(AudioFormat::Mp3, Some(96_000)), Some(48_000));
        // 44.1 kHz 源 → Opus 硬约束 48 kHz
        assert_eq!(recommended_sample_rate(AudioFormat::Opus, Some(44_100)), Some(48_000));
        // 192 kHz 源 → AAC 上限 96 kHz
        assert_eq!(recommended_sample_rate(AudioFormat::Aac, Some(192_000)), Some(96_000));
        // 源采样率未知 + Opus 仍要给硬约束值
        assert_eq!(recommended_sample_rate(AudioFormat::Opus, None), Some(48_000));
        // 低于所有候选（4 kHz）→ 回退首选
        assert_eq!(recommended_sample_rate(AudioFormat::Mp3, Some(4_000)), Some(44_100));
    }

    #[test]
    fn options_expose_single_recommendation_and_keep_source_first() {
        for format in [
            AudioFormat::Flac,
            AudioFormat::Wav,
            AudioFormat::Aiff,
            AudioFormat::Alac,
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Ogg,
            AudioFormat::Opus,
        ] {
            for source in [None, Some(44_100), Some(48_000), Some(96_000), Some(192_000)] {
                let options = sample_rate_options(format, source);
                assert!(options.len() >= 2, "{format:?}/{source:?} 至少要有“保持源”+1 个候选");
                assert!(options[0].value.is_none(), "第一项必须是保持源");
                assert_eq!(
                    options.iter().filter(|option| option.recommended).count(),
                    1,
                    "{format:?}/{source:?} 只能有一项被标为推荐"
                );
            }
        }
    }

    #[test]
    fn options_label_rates_and_warn_about_unsupported_source() {
        let options = sample_rate_options(AudioFormat::Flac, Some(44_100));
        assert!(options[0].label.contains("保持源"));
        assert!(options[0].label.contains("44 100"));
        assert!(options[0].recommended, "44.1 kHz 源转 FLAC 应推荐保持源");
        let high = options.iter().find(|o| o.value == Some(96_000)).unwrap();
        assert!(high.label.contains("96 000 Hz"));
        assert!(high.label.contains("高解析"));

        // Opus + 44.1 kHz 源：保持源不可行，推荐 48 kHz
        let opus = sample_rate_options(AudioFormat::Opus, Some(44_100));
        assert!(!opus[0].supported);
        assert!(opus[0].label.contains("不支持"));
        assert!(opus.iter().any(|o| o.value == Some(48_000) && o.recommended));
        assert!(opus[1].label.contains("Opus 原生"));
    }

    #[test]
    fn rate_text_is_grouped() {
        assert_eq!(format_rate(8_000), "8 000");
        assert_eq!(format_rate(44_100), "44 100");
        assert_eq!(format_rate(48_000), "48 000");
        assert_eq!(format_rate(384_000), "384 000");
    }

    fn media(bits: Option<u16>, rate: u32) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("in.flac"),
            format: Some(AudioFormat::Flac),
            container: "flac".into(),
            codec: "flac".into(),
            sample_rate: Some(rate),
            bits_per_sample: bits,
            is_float: false,
            channels: Some(2),
            channel_layout: Some("stereo".into()),
            duration_secs: Some(1.0),
            frames: Some(rate as u64),
            bit_rate: None,
            size_bytes: 0,
        }
    }

    #[test]
    fn defaults_match_design_table() {
        let src = media(Some(16), 44_100);

        let flac = default_spec(AudioFormat::Flac, &src).unwrap();
        // 设计 §15 风险 7：默认保持源位深（16-bit 源不再无谓升到 24-bit）
        assert_eq!(flac.bit_depth, Some(16));
        assert_eq!(flac.quality, EncodeQuality::FlacLevel { level: 8 });
        assert_eq!(flac.sample_rate, None);

        let wav = default_spec(AudioFormat::Wav, &src).unwrap();
        assert_eq!(wav.bit_depth, Some(16));
        assert_eq!(wav.quality, EncodeQuality::Pcm);

        let wav_hires = default_spec(AudioFormat::Wav, &media(Some(24), 96_000)).unwrap();
        assert_eq!(wav_hires.bit_depth, Some(24));

        assert_eq!(
            default_spec(AudioFormat::Mp3, &src).unwrap().quality,
            EncodeQuality::Mp3Vbr { q: 0 }
        );
        assert_eq!(
            default_spec(AudioFormat::Ogg, &src).unwrap().quality,
            EncodeQuality::OggQ { q: 6.0 }
        );
        assert_eq!(
            default_spec(AudioFormat::Opus, &src).unwrap().quality,
            EncodeQuality::OpusBitrate { kbps: 160 }
        );
        assert_eq!(
            default_spec(AudioFormat::Alac, &src).unwrap().bit_depth,
            Some(16)
        );
    }

    #[test]
    fn ape_and_dsd_outputs_are_rejected_with_unsupported() {
        for f in [AudioFormat::Ape, AudioFormat::Dsf, AudioFormat::Dff] {
            let err = default_spec(f, &media(Some(16), 44_100)).unwrap_err();
            assert_eq!(err.category(), tf_core::ErrorCategory::Unsupported);
        }
    }

    #[test]
    fn codec_args_cover_flac_pcm_and_lossy_defaults() {
        let flac = EncodeSpec {
            format: AudioFormat::Flac,
            bit_depth: Some(24),
            sample_rate: None,
            channels: None,
            quality: EncodeQuality::FlacLevel { level: 8 },
        };
        let args = codec_args(&flac).unwrap();
        assert!(args.contains(&"flac".to_string()));
        assert!(args.contains(&"s32".to_string()));
        assert_eq!(args[args.iter().position(|a| a == "-compression_level").unwrap() + 1], "8");

        let wav = EncodeSpec {
            format: AudioFormat::Wav,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            channels: Some(1),
            quality: EncodeQuality::Pcm,
        };
        let args = codec_args(&wav).unwrap();
        assert!(args.contains(&"pcm_s24le".to_string()));
        assert!(args.contains(&"-ar".to_string()));
        assert!(args.contains(&"48000".to_string()));
        assert!(args.contains(&"1".to_string()));

        let aiff = EncodeSpec {
            format: AudioFormat::Aiff,
            bit_depth: Some(16),
            ..wav.clone()
        };
        assert!(codec_args(&aiff).unwrap().contains(&"pcm_s16be".to_string()));

        let mp3 = EncodeSpec {
            format: AudioFormat::Mp3,
            bit_depth: None,
            sample_rate: None,
            channels: None,
            quality: EncodeQuality::Mp3Vbr { q: 0 },
        };
        let args = codec_args(&mp3).unwrap();
        assert!(args.contains(&"libmp3lame".to_string()));
        assert_eq!(args[args.iter().position(|a| a == "-q:a").unwrap() + 1], "0");

        let opus = EncodeSpec {
            quality: EncodeQuality::OpusBitrate { kbps: 192 },
            format: AudioFormat::Opus,
            ..mp3.clone()
        };
        let args = codec_args(&opus).unwrap();
        assert!(args.contains(&"192k".to_string()));

        let aac = EncodeSpec {
            format: AudioFormat::Aac,
            quality: EncodeQuality::AacBitrate { kbps: 256 },
            ..mp3.clone()
        };
        assert!(codec_args(&aac).unwrap().contains(&"256k".to_string()));
    }

    #[test]
    fn pcm_bit_depth_validation() {
        assert!(pcm_codec(24, false).is_ok());
        assert!(pcm_codec(20, false).is_err());
        let bogus = EncodeSpec {
            format: AudioFormat::Wav,
            bit_depth: Some(20),
            sample_rate: None,
            channels: None,
            quality: EncodeQuality::Pcm,
        };
        assert!(codec_args(&bogus).is_err());
    }

    #[test]
    fn pcm_writer_emits_interleaved_f32le() {
        let mut buf = AudioBuffer::new(48_000, 2, 3);
        buf.channel_mut(0).copy_from_slice(&[0.1, 0.2, 0.3]);
        buf.channel_mut(1).copy_from_slice(&[-0.1, -0.2, -0.3]);
        let mut writer = PcmWriter::new(&buf);
        let mut sink: Vec<u8> = Vec::new();
        writer.write_to(&mut sink).unwrap();
        assert_eq!(sink.len(), 3 * 2 * 4);
        let first = f32::from_le_bytes([sink[0], sink[1], sink[2], sink[3]]);
        assert!((first - 0.1f32).abs() < 1e-7);
        let second = f32::from_le_bytes([sink[4], sink[5], sink[6], sink[7]]);
        assert!((second + 0.1f32).abs() < 1e-7);
    }

    #[test]
    fn quality_summaries_are_human_readable() {
        assert!(EncodeQuality::Mp3Vbr { q: 0 }.summary().contains("V0"));
        assert!(EncodeQuality::OpusBitrate { kbps: 160 }
            .summary()
            .contains("160"));
    }

    #[test]
    fn missing_input_is_reported_as_input_error() {
        let paths = FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            source: crate::LocateSource::Path,
        };
        let spec = default_spec(AudioFormat::Flac, &media(Some(16), 44_100)).unwrap();
        let err = transcode(
            &paths,
            &spec,
            Path::new("C:/nope.flac"),
            Path::new("out.flac"),
            None,
            true,
            false,
            None,
        )
        .unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }

    // ---------------------------------------------------------- 无损默认位深 / 流转码 / soxr

    #[test]
    fn lossless_defaults_keep_source_bit_depth() {
        // 设计 §15 风险 7：16-bit 源不应被无谓升到 24-bit
        let cd = media(Some(16), 44_100);
        assert_eq!(default_spec(AudioFormat::Flac, &cd).unwrap().bit_depth, Some(16));
        assert_eq!(default_spec(AudioFormat::Alac, &cd).unwrap().bit_depth, Some(16));

        let hires = media(Some(24), 96_000);
        assert_eq!(default_spec(AudioFormat::Flac, &hires).unwrap().bit_depth, Some(24));

        // 源位深未知 / 32-bit float：回退 24
        assert_eq!(default_spec(AudioFormat::Flac, &media(None, 48_000)).unwrap().bit_depth, Some(24));
        assert_eq!(default_spec(AudioFormat::Flac, &media(Some(32), 192_000)).unwrap().bit_depth, Some(24));
    }

    #[test]
    fn stream_copy_requires_identical_target_spec() {
        let source = media(Some(16), 44_100);
        let mut source = source;
        source.format = Some(AudioFormat::Flac);
        source.channels = Some(2);

        let base = EncodeSpec {
            format: AudioFormat::Flac,
            bit_depth: None,
            sample_rate: None,
            channels: None,
            quality: EncodeQuality::FlacLevel { level: 8 },
        };
        // 完全保持源 → 可直接重封装（比特精确）
        assert!(can_stream_copy(&source, &base));
        // 明确写出与源相等的参数 → 仍可复制
        let same = EncodeSpec { bit_depth: Some(16), sample_rate: Some(44_100), channels: Some(2), ..base.clone() };
        assert!(can_stream_copy(&source, &same));
        // 降位深 → 必须走 DSP（否则无抖动）
        assert!(!can_stream_copy(&source, &EncodeSpec { bit_depth: Some(8), ..base.clone() }));
        // 改采样率/声道/格式 → 不能复制
        assert!(!can_stream_copy(&source, &EncodeSpec { sample_rate: Some(48_000), ..base.clone() }));
        assert!(!can_stream_copy(&source, &EncodeSpec { channels: Some(1), ..base.clone() }));
        assert!(!can_stream_copy(&source, &EncodeSpec { format: AudioFormat::Mp3, ..base.clone() }));
        // 不可编码格式（APE/DSD）永远不能作为输出
        let ape = EncodeSpec { format: AudioFormat::Ape, ..base.clone() };
        assert!(!can_stream_copy(&source, &ape));
        // 源位深未知（如 DSD）也不影响“保持源”的复制判定
        let mut unknown = source.clone();
        unknown.bits_per_sample = None;
        unknown.format = Some(AudioFormat::Wav);
        assert!(can_stream_copy(&unknown, &EncodeSpec { format: AudioFormat::Wav, ..base }));
    }

    #[test]
    fn stream_copy_args_never_reencode() {
        let args = stream_copy_args(Path::new("in.flac"), Path::new("out.flac"));
        let joined = args.join(" ");
        assert!(joined.contains("-c:a copy"), "{joined}");
        assert!(joined.contains("-map 0:a:0"), "{joined}");
        assert!(joined.contains("-map_metadata -1"), "不写 ffmpeg 标签：{joined}");
        assert!(!joined.contains("-ar"), "重封装不得重采样：{joined}");
        assert!(!joined.contains("-ac"), "重封装不得改声道：{joined}");
    }

    #[test]
    fn soxr_is_used_only_when_needed_and_available() {
        let caps_with = Capabilities {
            version_line: String::new(),
            configuration: "--enable-libsoxr --enable-libmp3lame".into(),
            encoders: Vec::new(),
            raw_encoders: String::new(),
        };
        let caps_without = Capabilities { configuration: "--disable-everything".into(), ..caps_with.clone() };
        assert!(has_soxr(&caps_with));
        assert!(!has_soxr(&caps_without));

        let spec = EncodeSpec {
            format: AudioFormat::Flac,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            channels: None,
            quality: EncodeQuality::FlacLevel { level: 8 },
        };
        let soxr = resample_args(&spec, true).join(" ");
        assert!(soxr.contains("aresample=osr=48000:resampler=soxr"), "{soxr}");
        let plain = resample_args(&spec, false).join(" ");
        assert_eq!(plain, "-ar 48000");
        // 不需要重采样时不能插入重采样滤镜
        let keep = EncodeSpec { sample_rate: None, ..spec };
        assert!(resample_args(&keep, true).is_empty());
        assert!(resample_args(&keep, false).is_empty());
    }
}
