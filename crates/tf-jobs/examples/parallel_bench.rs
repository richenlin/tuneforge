//! 并发基线测量（开发工具，不参与打包）。
//!
//! 用随包 ffmpeg 生成一批测试音频，然后在不同并发度下跑真实流水线，打印耗时，
//! 用来验证「批量转换是否真的吃满了多核」以及回归对比。
//!
//! ```text
//! cargo run -p tf-jobs --release --example parallel_bench
//! cargo run -p tf-jobs --release --example parallel_bench -- 8 120 normalize
//! ```
//!
//! 参数：`[文件数=8] [每个文件秒数=120] [convert|normalize]`。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use tf_jobs::{
    plan_convert, plan_normalize, ChannelMode, ConflictPolicy, ConvertConfig, ConvertSource,
    FfmpegPipeline, JobPlan, JobReport, NormalizeConfig, NullSink, QueueRunner,
};
use tf_media::{probe, FfmpegPaths};

fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
    let seconds: f64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(120.0);
    let mode = args.next().unwrap_or_else(|| "convert".into());

    let Some(paths) = bundled_paths() else {
        eprintln!(
            "找不到随包 ffmpeg/ffprobe（app/src-tauri/binaries），先跑 scripts/fetch-ffmpeg.ps1"
        );
        std::process::exit(2);
    };

    let root = std::env::temp_dir().join("tuneforge-parallel-bench");
    let _ = std::fs::remove_dir_all(&root);
    let inputs_dir = root.join("in");
    std::fs::create_dir_all(&inputs_dir).expect("创建测试输入目录");

    println!("生成 {count} 个 {seconds:.0} 秒 FLAC…");
    let setup_started = Instant::now();
    let sources = generate_inputs(&paths, &inputs_dir, count, seconds);
    println!(
        "输入就绪：{} 个文件，用时 {:.1} s\n",
        sources.len(),
        setup_started.elapsed().as_secs_f64()
    );

    println!(
        "CPU 核心数：{}，默认并发度：{}\n",
        tf_jobs::queue::cpu_cores(),
        tf_jobs::default_concurrency()
    );
    println!(
        "{:>10}  {:>10}  {:>10}  {:>10}",
        "并发", "耗时(s)", "吞吐(文件/s)", "相对加速"
    );
    let mut baseline = 0.0f64;
    for concurrency in [1usize, 4, 8, 16] {
        let out_dir = root.join(format!("out-{concurrency}"));
        std::fs::create_dir_all(&out_dir).expect("创建输出目录");
        let Some(plan) = build_plan(&mode, &sources, &out_dir) else {
            eprintln!("未知模式：{mode}（可用 convert / normalize）");
            std::process::exit(2);
        };
        let report = run_once(&paths, &plan, concurrency);
        let secs = report.elapsed_ms as f64 / 1000.0;
        if concurrency == 1 {
            baseline = secs;
        }
        let speedup = if baseline > 0.0 { baseline / secs } else { 1.0 };
        println!(
            "{:>10}  {:>10.2}  {:>10.2}  {:>9.2}x",
            report.concurrency,
            secs,
            plan.len() as f64 / secs,
            speedup
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

fn build_plan(mode: &str, sources: &[ConvertSource], out_dir: &Path) -> Option<JobPlan> {
    match mode {
        "convert" => {
            let config = ConvertConfig {
                // FLAC → MP3 是典型的「纯转码」批处理：ffmpeg 一条命令搞定。
                spec: tf_media::default_spec(tf_core::model::AudioFormat::Mp3, &sources[0].media)
                    .ok()?,
                keep_tags: false,
                gain_db: None,
                ceiling_dbtp: None,
                channel_mode: ChannelMode::Keep,
                downmix_fake_multichannel: false,
            };
            plan_convert(
                sources.to_vec(),
                &config,
                out_dir,
                ConflictPolicy::Overwrite,
            )
            .ok()
        }
        "normalize" => {
            let config = NormalizeConfig {
                params: tf_core::limiter::NormalizeParams::default(),
                // 归一化必须走 DSP（解码到内存 → 测量/限幅 → 编码）。
                spec: tf_media::default_spec(tf_core::model::AudioFormat::Flac, &sources[0].media)
                    .ok()?,
                keep_tags: false,
            };
            plan_normalize(
                sources.to_vec(),
                &config,
                out_dir,
                ConflictPolicy::Overwrite,
            )
            .ok()
        }
        _ => None,
    }
}

fn run_once(paths: &FfmpegPaths, plan: &JobPlan, concurrency: usize) -> JobReport {
    let runner = QueueRunner::new(concurrency);
    let pipeline = FfmpegPipeline::new(paths.clone()).with_threads(runner.child_thread_budget());
    runner
        .run(
            plan,
            &pipeline,
            Arc::new(NullSink),
            tf_jobs::CancelToken::new(),
        )
        .expect("任务执行失败")
}

fn generate_inputs(
    paths: &FfmpegPaths,
    dir: &Path,
    count: usize,
    seconds: f64,
) -> Vec<ConvertSource> {
    (0..count)
        .map(|index| {
            let file = dir.join(format!("track-{index:02}.flac"));
            let status = Command::new(&paths.ffmpeg)
                .args([
                    "-hide_banner",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency={}:duration={seconds}", 220 + index * 30),
                    "-ar",
                    "44100",
                    "-ac",
                    "2",
                    "-c:a",
                    "flac",
                    "-y",
                ])
                .arg(&file)
                .status()
                .expect("启动 ffmpeg 生成测试文件失败");
            assert!(status.success(), "生成测试文件失败：{}", file.display());
            let media = probe::read_media_info(paths, &file).expect("探测测试文件失败");
            ConvertSource { path: file, media }
        })
        .collect()
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
