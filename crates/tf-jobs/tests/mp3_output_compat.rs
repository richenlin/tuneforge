//! 端到端（真实 ffmpeg）：转换/归一化写出的 MP3 必须是 **ID3v2.3**（老播放器兼容），
//! 且在未指定采样率时保持源采样率（不做无谓重采样）。
//!
//! 依赖随包分发的 ffmpeg/ffprobe（`app/src-tauri/binaries`）；找不到时用例自行跳过，
//! 这样没有二进制的环境仍然通过。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tf_jobs::{
    plan_convert, ChannelMode, ConflictPolicy, ConvertConfig, ConvertSource, FfmpegPipeline,
    JobContext, LogLevel, Pipeline, Reporter,
};
use tf_media::FfmpegPaths;

struct NullReporter;

impl Reporter for NullReporter {
    fn progress(&self, _percent: f32, _stage: &str) {}
    fn log(&self, _level: LogLevel, _message: &str) {}
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// 找出随包分发的 ffmpeg / ffprobe（sidecar 命名带 target triple）。
fn bundled_paths() -> Option<FfmpegPaths> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/binaries");
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let mut ffmpeg: Option<PathBuf> = None;
    let mut ffprobe: Option<PathBuf> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("ffmpeg") && name.ends_with(suffix) {
            ffmpeg = Some(entry.path());
        }
        if name.starts_with("ffprobe") && name.ends_with(suffix) {
            ffprobe = Some(entry.path());
        }
    }
    Some(FfmpegPaths {
        ffmpeg: ffmpeg?,
        ffprobe: ffprobe?,
        source: tf_media::LocateSource::BundledSidecar,
    })
}

/// 文件里 ID3v2 标签的主版本号（跳过 RIFF 的 `ID3 ` chunk 标识）。
fn id3_major_version(bytes: &[u8]) -> Option<u8> {
    for index in 0..bytes.len().saturating_sub(5) {
        if &bytes[index..index + 3] == b"ID3"
            && bytes[index + 4] == 0
            && matches!(bytes[index + 3], 3 | 4)
        {
            return Some(bytes[index + 3]);
        }
    }
    None
}

#[test]
fn mp3_output_is_id3v23_and_keeps_source_rate() {
    let Some(paths) = bundled_paths() else {
        eprintln!("跳过：未找到随包 ffmpeg/ffprobe");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("src.flac");
    // 44.1 kHz 立体声 + 中文标签
    let ok = tf_media::process::command(&paths.ffmpeg)
        .args([
            "-hide_banner", "-v", "error", "-f", "lavfi", "-i",
            "sine=frequency=440:duration=2", "-ar", "44100", "-ac", "2", "-c:a", "flac",
            "-metadata", "title=黄昏", "-metadata", "artist=周传雄", "-y",
        ])
        .arg(&input)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "生成测试 FLAC 失败");

    let media = tf_media::read_media_info(&paths, &input).unwrap();
    assert_eq!(media.sample_rate, Some(44_100));

    let spec = tf_media::default_spec(tf_core::model::AudioFormat::Mp3, &media).unwrap();
    assert_eq!(spec.sample_rate, None, "默认应保持源采样率（不重采样）");

    let config = ConvertConfig {
        spec,
        keep_tags: true,
        gain_db: None,
        ceiling_dbtp: Some(-1.0),
        channel_mode: ChannelMode::Keep,
        downmix_fake_multichannel: false,
    };
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let plan = plan_convert(
        vec![ConvertSource {
            path: input.clone(),
            media,
        }],
        &config,
        &out_dir,
        ConflictPolicy::Skip,
    )
    .unwrap();
    let job = plan.jobs.into_iter().next().unwrap();
    let target = out_dir.join(&job.output_name);

    let pipeline = FfmpegPipeline::new(paths.clone());
    let ctx = JobContext {
        job: &job,
        output_dir: &out_dir,
        reporter: Arc::new(NullReporter),
    };
    let report = pipeline.produce(&job, &target, &ctx).unwrap();
    assert!(target.is_file(), "输出文件应存在：{}", target.display());
    assert!(report.warnings.is_empty(), "不应有告警：{:?}", report.warnings);

    // 标签必须是 ID3v2.3（老播放器/车载能认），且中文标签不丢
    let bytes = std::fs::read(&target).unwrap();
    assert_eq!(
        id3_major_version(&bytes),
        Some(3),
        "MP3 输出应为 ID3v2.3（当前 {}", 
        id3_major_version(&bytes).map_or("无标签".into(), |v| format!("v2.{v}"))
    );
    let tags = tf_tags::read_tags(&target).unwrap();
    assert_eq!(tags.title.as_deref(), Some("黄昏"));
    assert_eq!(tags.artist.as_deref(), Some("周传雄"));

    // 未指定采样率时输出应保持源采样率（44.1 kHz 不该被重采样成 48 kHz）
    let out_media = tf_media::read_media_info(&paths, &target).unwrap();
    assert_eq!(out_media.sample_rate, Some(44_100), "应保持源采样率");
}
