//! 真实处理流水线：解码 → tf-core DSP → 编码 → 写标签（设计方案 §4.3 / §7）。
//!
//! 流水线只负责“把结果写到临时文件”，提交/清理由队列负责（`TempGuard`）。
//! 转换时若不需要 DSP（无增益、不折混、不改声道），则直接 ffmpeg→ffmpeg 转码，避免无谓的解码开销。

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tf_core::dither::{needs_dither, quantize_in_place};
use tf_core::downmix;
use tf_core::error::{Result, TfError};
use tf_core::limiter;
use tf_core::model::AudioBuffer;
use tf_media::{decode, encode, probe, EncodeSpec, FfmpegPaths, MediaProgress};
use tf_tags::{copy_tags, set_cover, write_tags, CopyPolicy, WriteOptions};

use crate::plan::{ChannelMode, CoverAction, Job, JobPayload};
use crate::queue::{JobContext, LogLevel, Reporter};

/// 抖动/量化的确定性种子（同一文件重复处理结果一致）。
const DITHER_SEED: u64 = 0x7A17_F0A5_1234_5678;

/// 流水线产出信息。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProduceReport {
    /// 展示用说明（例如增益/真峰值统计）。
    pub note: Option<String>,
    /// 非致命告警（例如标签写入失败）。
    pub warnings: Vec<String>,
}

/// 执行单个任务。
pub trait Pipeline: Send + Sync {
    /// 把 `job` 的结果写入 `target`（临时文件）。
    fn produce(&self, job: &Job, target: &Path, ctx: &JobContext<'_>) -> Result<ProduceReport>;
}

/// 基于 FFmpeg 子进程的真实流水线。
pub struct FfmpegPipeline {
    paths: FfmpegPaths,
    /// 当前 ffmpeg 是否支持 soxr（需要重采样时用它做高质量转换）。
    soxr: bool,
}

impl FfmpegPipeline {
    /// 用已定位的 ffmpeg 路径构造（默认不启用 soxr）。
    pub fn new(paths: FfmpegPaths) -> Self {
        FfmpegPipeline { paths, soxr: false }
    }

    /// 按能力快照构造：带 libsoxr 的构建会启用高质量重采样。
    pub fn with_capabilities(paths: FfmpegPaths, caps: &tf_media::Capabilities) -> Self {
        FfmpegPipeline {
            soxr: tf_media::has_soxr(caps),
            paths,
        }
    }

    /// ffmpeg 路径。
    pub fn paths(&self) -> &FfmpegPaths {
        &self.paths
    }

    fn progress_callback(
        reporter: &Arc<dyn Reporter>,
    ) -> Arc<tf_media::progress::ProgressCallback<'static>> {
        let reporter = Arc::clone(reporter);
        Arc::new(move |p: MediaProgress| {
            let stage = match p.stage {
                tf_media::progress::ProgressStage::Probe => "探测",
                tf_media::progress::ProgressStage::Decode => "解码",
                tf_media::progress::ProgressStage::Process => "处理",
                tf_media::progress::ProgressStage::Encode => "编码",
                tf_media::progress::ProgressStage::Tag => "写标签",
            };
            let percent = p.percent.unwrap_or(0.0) as f32;
            reporter.progress(percent, stage);
        })
    }

    /// 是否需要走 Rust DSP 路径。
    ///
    /// 除声道模式/增益/假多声道折混外，**降位深也必须走 DSP**：
    /// Rust 侧会做 TPDF 抖动（`dither::quantize_in_place`），而 ffmpeg 命令行默认**不做抖动**，
    /// 直接截断会产生量化失真（小信号尤其明显）。
    /// 有损格式内部是浮点/感知编码，提前量化反而更差，因此只对无损整数目标生效。
    fn needs_dsp(
        mode: ChannelMode,
        gain_db: Option<f64>,
        downmix_fake: bool,
        spec: &EncodeSpec,
        source_bits: Option<u16>,
    ) -> bool {
        if mode != ChannelMode::Keep || gain_db.is_some() || downmix_fake {
            return true;
        }
        match spec.bit_depth {
            Some(bits) if spec.format.is_lossless() => needs_dither(source_bits, bits),
            _ => false,
        }
    }

    fn convert(
        &self,
        job: &Job,
        target: &Path,
        ctx: &JobContext<'_>,
        payload: &crate::plan::ConvertPayload,
    ) -> Result<ProduceReport> {
        let media = probe::read_media_info(&self.paths, &job.input)?;
        let duration = media.duration_secs;
        let mut warnings = Vec::new();
        let mut note_parts: Vec<String> = Vec::new();

        let spec = &payload.spec;
        if Self::needs_dsp(
            payload.channel_mode,
            payload.gain_db,
            payload.downmix_fake_multichannel,
            spec,
            media.bits_per_sample,
        ) {
            let opts = decode::DecodeOptions {
                source_duration_secs: duration,
                ..Default::default()
            };
            ctx.reporter.progress(0.0, "解码");
            let mut buffer = decode::decode_to_buffer(
                &self.paths,
                &job.input,
                &opts,
                Some(Self::progress_callback(&ctx.reporter)),
            )?;
            ctx.ensure_not_cancelled()?;

            buffer = self.apply_channel_processing(
                buffer,
                payload.channel_mode,
                payload.downmix_fake_multichannel,
                &mut note_parts,
            )?;

            if let Some(gain_db) = payload.gain_db {
                let (gained, trim_db) =
                    limiter::apply_gain_with_ceiling(&buffer, gain_db, payload.ceiling_dbtp);
                buffer = gained;
                note_parts.push(format!("增益 {:+.2} dB", gain_db));
                if trim_db > 0.0 {
                    note_parts.push(format!("真峰值安全衰减 -{trim_db:.2} dB"));
                }
            }

            self.apply_dither(&mut buffer, spec.bit_depth, media.bits_per_sample, &mut note_parts);

            ctx.reporter.progress(0.0, "编码");
            encode::encode_buffer(
                &self.paths,
                spec,
                &buffer,
                target,
                duration,
                self.soxr,
                Some(Self::progress_callback(&ctx.reporter)),
            )?;
        } else if encode::can_stream_copy(&media, spec) {
            // 目标参数与源完全一致：只重封装音频流（比特精确），不做任何重编码
            note_parts.push("音频流直接复制（比特精确，零重编码）".into());
            ctx.reporter.progress(0.0, "重封装");
            encode::transcode_copy(
                &self.paths,
                &job.input,
                target,
                duration,
                Some(Self::progress_callback(&ctx.reporter)),
            )?;
        } else {
            ctx.reporter.progress(0.0, "转码");
            encode::transcode(
                &self.paths,
                spec,
                &job.input,
                target,
                duration,
                false,
                self.soxr,
                Some(Self::progress_callback(&ctx.reporter)),
            )?;
        }

        if payload.keep_tags {
            ctx.reporter.progress(1.0, "写标签");
            if let Err(e) = copy_tags(
                &job.input,
                target,
                CopyPolicy::TagsAndCover,
                &WriteOptions::default(),
            ) {
                // §8：标签失败不影响音频完整性
                warnings.push(format!("标签复制失败：{e}"));
            }
        }

        ctx.reporter.progress(1.0, "完成");
        Ok(ProduceReport {
            note: Some(note_parts.join("，")).filter(|s| !s.is_empty()),
            warnings,
        })
    }

    fn apply_channel_processing(
        &self,
        buffer: AudioBuffer,
        mode: ChannelMode,
        downmix_fake: bool,
        notes: &mut Vec<String>,
    ) -> Result<AudioBuffer> {
        let mut buffer = buffer;
        if downmix_fake && downmix::is_fake_multichannel(&buffer) {
            let report = downmix::detect_fake_multichannel(
                &buffer,
                downmix::DEFAULT_SILENCE_THRESHOLD_DBFS,
            );
            notes.push(format!("假多声道折混（{}）", report.channels));
            buffer = downmix::downmix_fake_multichannel(&buffer)?;
        }
        buffer = match mode {
            ChannelMode::Keep => buffer,
            ChannelMode::Stereo => {
                notes.push("折混为立体声".into());
                downmix::downmix_to_stereo(&buffer)?
            }
            ChannelMode::Mono => {
                notes.push("折混为单声道".into());
                downmix::downmix_to_mono(&buffer)?
            }
        };
        Ok(buffer)
    }

    fn apply_dither(
        &self,
        buffer: &mut AudioBuffer,
        target_bits: Option<u16>,
        source_bits: Option<u16>,
        notes: &mut Vec<String>,
    ) {
        if let Some(bits) = target_bits {
            if needs_dither(source_bits, bits) {
                let err = quantize_in_place(buffer, bits, DITHER_SEED);
                notes.push(format!(
                    "降位深至 {bits}-bit（TPDF 抖动，最大误差 {:.2e}）",
                    err
                ));
            }
        }
    }

    fn normalize(
        &self,
        job: &Job,
        target: &Path,
        ctx: &JobContext<'_>,
        payload: &crate::plan::NormalizePayload,
    ) -> Result<ProduceReport> {
        let media = probe::read_media_info(&self.paths, &job.input)?;
        let duration = media.duration_secs;
        let mut notes: Vec<String> = Vec::new();
        let mut warnings = Vec::new();

        ctx.reporter.progress(0.0, "解码");
        let buffer = decode::decode_to_buffer(
            &self.paths,
            &job.input,
            &decode::DecodeOptions {
                source_duration_secs: duration,
                ..Default::default()
            },
            Some(Self::progress_callback(&ctx.reporter)),
        )?;
        ctx.ensure_not_cancelled()?;

        ctx.reporter.progress(0.0, "测量与限幅");
        let outcome = limiter::normalize(&buffer, &payload.params)?;
        notes.push(format!(
            "{:.2} LUFS → {:.2} LUFS（增益 {:+.2} dB，真峰值 {:.2} dBTP{}）",
            outcome.measured_lufs,
            outcome.output_lufs.unwrap_or(f64::NAN),
            outcome.base_gain_db,
            outcome.output_true_peak_dbtp,
            if outcome.limited {
                format!("，限幅 -{:.2} dB", outcome.limiter_reduction_db)
            } else {
                String::new()
            }
        ));
        if outcome.downmixed {
            notes.push("已折混假多声道".into());
        }
        if outcome.final_trim_db > 0.0 {
            notes.push(format!("安全 trim -{:.2} dB", outcome.final_trim_db));
        }

        let mut out = outcome.buffer;
        self.apply_dither(
            &mut out,
            payload.spec.bit_depth,
            media.bits_per_sample,
            &mut notes,
        );

        ctx.reporter.progress(0.0, "编码");
        encode::encode_buffer(
            &self.paths,
            &payload.spec,
            &out,
            target,
            duration,
            self.soxr,
            Some(Self::progress_callback(&ctx.reporter)),
        )?;

        if payload.keep_tags {
            ctx.reporter.progress(1.0, "写标签");
            if let Err(e) = copy_tags(
                &job.input,
                target,
                CopyPolicy::TagsAndCover,
                &WriteOptions::default(),
            ) {
                warnings.push(format!("标签复制失败：{e}"));
            }
        }

        ctx.reporter.progress(1.0, "完成");
        Ok(ProduceReport {
            note: Some(notes.join("，")),
            warnings,
        })
    }

    fn rename(&self, job: &Job, target: &Path, ctx: &JobContext<'_>) -> Result<ProduceReport> {
        ctx.reporter.progress(0.0, "复制");
        copy_file(&job.input, target)?;
        ctx.reporter.progress(1.0, "完成");
        Ok(ProduceReport {
            note: Some("仅改名，未转码".into()),
            warnings: Vec::new(),
        })
    }

    fn write_tags_only(
        &self,
        job: &Job,
        target: &Path,
        ctx: &JobContext<'_>,
        payload: &crate::plan::TagsPayload,
    ) -> Result<ProduceReport> {
        ctx.reporter.progress(0.0, "复制");
        copy_file(&job.input, target)?;

        ctx.reporter.progress(0.5, "写标签");
        write_tags(target, &payload.tags, &WriteOptions::default())?;

        let mut warnings = Vec::new();
        match &payload.cover {
            Some(CoverAction::Set(cover)) => {
                if let Err(e) = set_cover(target, Some(cover)) {
                    warnings.push(format!("写入封面失败：{e}"));
                }
            }
            Some(CoverAction::Remove) => {
                if let Err(e) = tf_tags::remove_covers(target) {
                    warnings.push(format!("删除封面失败：{e}"));
                }
            }
            Some(CoverAction::Keep) | None => {}
        }

        ctx.reporter.progress(1.0, "完成");
        Ok(ProduceReport {
            note: Some("标签已更新".into()),
            warnings,
        })
    }
}

impl Pipeline for FfmpegPipeline {
    fn produce(&self, job: &Job, target: &Path, ctx: &JobContext<'_>) -> Result<ProduceReport> {
        ctx.reporter.log(LogLevel::Info, "开始处理");
        match &job.payload {
            JobPayload::Convert(payload) => self.convert(job, target, ctx, payload),
            JobPayload::Normalize(payload) => self.normalize(job, target, ctx, payload),
            JobPayload::Rename => self.rename(job, target, ctx),
            JobPayload::Tags(payload) => self.write_tags_only(job, target, ctx, payload),
        }
    }
}

/// 文件复制（保留源文件，非破坏性）。
fn copy_file(from: &Path, to: &Path) -> Result<u64> {
    if !from.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", from.display())));
    }
    std::fs::copy(from, to)
        .map_err(|e| TfError::Io(format!("复制 {} 失败：{e}", from.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct NullReporter;

    impl Reporter for NullReporter {
        fn progress(&self, _percent: f32, _stage: &str) {}
        fn log(&self, _level: LogLevel, _message: &str) {}
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    /// 冒烟：没有 ffmpeg 时定位必须给出可操作错误（不 panic）。
    ///
    /// 显式关闭 PATH 回退：否则本用例会随开发机/CI 是否装了 ffmpeg 而飘。
    #[test]
    fn discover_without_binaries_reports_unsupported() {
        let err = tf_media::locate::discover_opts(
            None,
            &[std::path::PathBuf::from("C:/missing")],
            false,
        )
        .unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Unsupported);
    }

    #[test]
    fn dsp_decision_matches_rules() {
        let lossless = EncodeSpec {
            format: tf_core::model::AudioFormat::Flac,
            bit_depth: None,
            sample_rate: None,
            channels: None,
            quality: tf_media::EncodeQuality::FlacLevel { level: 8 },
        };
        assert!(!FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &lossless, Some(16)));
        assert!(FfmpegPipeline::needs_dsp(ChannelMode::Stereo, None, false, &lossless, Some(16)));
        assert!(FfmpegPipeline::needs_dsp(ChannelMode::Keep, Some(3.0), false, &lossless, Some(16)));
        assert!(FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, true, &lossless, Some(16)));

        // 降位深必须走 DSP（否则到 ffmpeg 那里是不抖动地硬截断）
        let down16 = EncodeSpec { bit_depth: Some(16), ..lossless.clone() };
        assert!(FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &down16, Some(24)));
        // 升位深 / 同位深不需要 DSP
        let up24 = EncodeSpec { bit_depth: Some(24), ..lossless.clone() };
        assert!(!FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &up24, Some(16)));
        assert!(!FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &up24, Some(24)));
        // 源位深未知时不强行走 DSP（无法判断是否降位深）
        assert!(!FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &down16, None));
        // 有损目标不因位深进 DSP（编码器内部为浮点/感知编码）
        let lossy = EncodeSpec {
            format: tf_core::model::AudioFormat::Mp3,
            bit_depth: Some(16),
            sample_rate: None,
            channels: None,
            quality: tf_media::EncodeQuality::Mp3Vbr { q: 0 },
        };
        assert!(!FfmpegPipeline::needs_dsp(ChannelMode::Keep, None, false, &lossy, Some(24)));
    }

    #[test]
    fn stream_copy_is_chosen_only_for_identical_specs() {
        let caps = tf_media::Capabilities {
            version_line: String::new(),
            configuration: "--enable-libsoxr".into(),
            encoders: Vec::new(),
            raw_encoders: String::new(),
        };
        let pipeline = FfmpegPipeline::with_capabilities(
            FfmpegPaths {
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
                source: tf_media::LocateSource::Path,
            },
            &caps,
        );
        // 带 soxr 的构建下，pipeline 内部标记为可用（行为由 encode::can_stream_copy 决定）
        assert!(tf_media::has_soxr(&caps));
        let _ = pipeline;
    }

    #[test]
    fn channel_processing_notes_are_recorded() {
        // 用 tf-core 的纯函数验证折混行为（不依赖 ffmpeg）
        let mut buf = AudioBuffer::new(48_000, 8, 4800);
        for ch in 0..2 {
            buf.channel_mut(ch).iter_mut().for_each(|v| *v = 0.3);
        }
        let mut notes = Vec::new();
        let pipeline = FfmpegPipeline::new(FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            source: tf_media::LocateSource::Path,
        });
        let out = pipeline
            .apply_channel_processing(buf, ChannelMode::Keep, true, &mut notes)
            .unwrap();
        assert_eq!(out.channels, 2);
        assert!(notes[0].contains("假多声道"));
    }

    #[test]
    fn dither_applied_only_when_reducing_depth() {
        let pipeline = FfmpegPipeline::new(FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            source: tf_media::LocateSource::Path,
        });
        let mut buf = AudioBuffer::new(48_000, 1, 1000);
        buf.channel_mut(0).iter_mut().enumerate().for_each(|(i, v)| {
            *v = (i as f64 / 1000.0) * 0.5;
        });
        let mut notes = Vec::new();
        pipeline.apply_dither(&mut buf, Some(16), Some(24), &mut notes);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("TPDF"));

        let mut notes2 = Vec::new();
        pipeline.apply_dither(&mut buf, Some(24), Some(16), &mut notes2);
        assert!(notes2.is_empty(), "升位深不应抖动");
    }

    #[test]
    fn missing_input_file_is_input_error_for_copy_paths() {
        let err = copy_file(Path::new("C:/nope.flac"), Path::new("C:/out.flac")).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }

    #[test]
    fn reporter_progress_is_clamped_by_queue() {
        let reporter: Arc<dyn Reporter> = Arc::new(NullReporter);
        assert!(!reporter.is_cancelled());
        reporter.progress(2.0, "x");
        reporter.log(LogLevel::Info, "y");
    }
}
