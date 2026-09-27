//! 任务队列：并发执行、进度事件、取消（设计方案 §10.4 / §10.5）。

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

use serde::Serialize;
use tf_core::error::{Result, TfError};

use crate::output::{OutputDecision, OutputResolver, TempGuard};
use crate::pipeline::Pipeline;
use crate::plan::{Job, JobPlan};
use crate::report::{JobReport, JobResult, TaskStatus};

/// 默认并发度：`min(CPU 核心数, 4)`（FFmpeg 自身也可能多线程，避免过载）。
pub fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 4)
}

/// 取消令牌（可克隆，跨线程共享）。
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// 新建未取消的令牌。
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// 请求取消。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// 是否已请求取消。
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 已取消则返回 [`TfError::Cancelled`]。
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(TfError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// 调试。
    Debug,
    /// 信息。
    Info,
    /// 警告。
    Warn,
    /// 错误。
    Error,
}

impl LogLevel {
    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Debug => "调试",
            LogLevel::Info => "信息",
            LogLevel::Warn => "警告",
            LogLevel::Error => "错误",
        }
    }
}

/// 队列事件（UI 通过 Tauri 事件推送）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum QueueEvent {
    /// 任务开始。
    Started {
        /// 任务 id。
        job_id: String,
        /// 展示标签。
        label: String,
        /// 源文件。
        input: String,
    },
    /// 单文件进度。
    Progress {
        /// 任务 id。
        job_id: String,
        /// 百分比 0..=1。
        percent: f32,
        /// 阶段说明。
        stage: String,
    },
    /// 日志行。
    Log {
        /// 任务 id。
        job_id: String,
        /// 级别。
        level: LogLevel,
        /// 内容。
        message: String,
    },
    /// 任务结束。
    Finished {
        /// 结果。
        result: JobResult,
    },
}

/// 事件接收端（Tauri 后端实现为向前端 emit）。
pub trait EventSink: Send + Sync {
    /// 任务开始。
    fn on_job_started(&self, _job: &Job) {}
    /// 单文件进度。
    fn on_job_progress(&self, _job_id: &str, _percent: f32, _stage: &str) {}
    /// 日志。
    fn on_job_log(&self, _job_id: &str, _level: LogLevel, _message: &str) {}
    /// 任务结束。
    fn on_job_finished(&self, _result: &JobResult) {}
}

/// 什么都不做的接收端。
#[derive(Debug, Default)]
pub struct NullSink;

impl EventSink for NullSink {}

/// 把事件收集到内存（测试与结果页使用）。
#[derive(Debug, Default)]
pub struct CollectingSink {
    events: Mutex<Vec<QueueEvent>>,
}

impl CollectingSink {
    /// 新建。
    pub fn new() -> Self {
        CollectingSink::default()
    }

    /// 取出全部事件快照。
    pub fn events(&self) -> Vec<QueueEvent> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 已结束的任务数。
    pub fn finished_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|e| matches!(e, QueueEvent::Finished { .. }))
            .count()
    }

    /// 进度事件数。
    pub fn progress_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|e| matches!(e, QueueEvent::Progress { .. }))
            .count()
    }

    /// 日志内容。
    pub fn log_messages(&self) -> Vec<String> {
        self.events()
            .iter()
            .filter_map(|e| match e {
                QueueEvent::Log { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    fn push(&self, event: QueueEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
    }
}

impl EventSink for CollectingSink {
    fn on_job_started(&self, job: &Job) {
        self.push(QueueEvent::Started {
            job_id: job.id.clone(),
            label: job.label.clone(),
            input: job.input.display().to_string(),
        });
    }

    fn on_job_progress(&self, job_id: &str, percent: f32, stage: &str) {
        self.push(QueueEvent::Progress {
            job_id: job_id.to_string(),
            percent,
            stage: stage.to_string(),
        });
    }

    fn on_job_log(&self, job_id: &str, level: LogLevel, message: &str) {
        self.push(QueueEvent::Log {
            job_id: job_id.to_string(),
            level,
            message: message.to_string(),
        });
    }

    fn on_job_finished(&self, result: &JobResult) {
        self.push(QueueEvent::Finished {
            result: result.clone(),
        });
    }
}

/// 转发事件到闭包。
pub struct FnSink<F>
where
    F: Fn(QueueEvent) + Send + Sync,
{
    f: F,
}

impl<F> FnSink<F>
where
    F: Fn(QueueEvent) + Send + Sync,
{
    /// 新建。
    pub fn new(f: F) -> Self {
        FnSink { f }
    }
}

impl<F> EventSink for FnSink<F>
where
    F: Fn(QueueEvent) + Send + Sync,
{
    fn on_job_started(&self, job: &Job) {
        (self.f)(QueueEvent::Started {
            job_id: job.id.clone(),
            label: job.label.clone(),
            input: job.input.display().to_string(),
        });
    }

    fn on_job_progress(&self, job_id: &str, percent: f32, stage: &str) {
        (self.f)(QueueEvent::Progress {
            job_id: job_id.to_string(),
            percent,
            stage: stage.to_string(),
        });
    }

    fn on_job_log(&self, job_id: &str, level: LogLevel, message: &str) {
        (self.f)(QueueEvent::Log {
            job_id: job_id.to_string(),
            level,
            message: message.to_string(),
        });
    }

    fn on_job_finished(&self, result: &JobResult) {
        (self.f)(QueueEvent::Finished {
            result: result.clone(),
        });
    }
}

/// 交给流水线的上报接口。
pub trait Reporter: Send + Sync {
    /// 单文件进度（0..=1）与阶段说明。
    fn progress(&self, percent: f32, stage: &str);
    /// 日志。
    fn log(&self, level: LogLevel, message: &str);
    /// 是否已取消（长任务应在阶段边界检查）。
    fn is_cancelled(&self) -> bool;
}

/// 单个任务的执行上下文。
pub struct JobContext<'a> {
    /// 当前任务。
    pub job: &'a Job,
    /// 输出目录。
    pub output_dir: &'a Path,
    /// 上报接口（可跨线程持有，流水线内部实现需要 `'static` 回调）。
    pub reporter: Arc<dyn Reporter>,
}

impl JobContext<'_> {
    /// 已取消则返回 [`TfError::Cancelled`]。
    pub fn ensure_not_cancelled(&self) -> Result<()> {
        if self.reporter.is_cancelled() {
            Err(TfError::Cancelled)
        } else {
            Ok(())
        }
    }
}

struct QueueReporter {
    job_id: String,
    sink: Arc<dyn EventSink>,
    cancel: CancelToken,
}

impl Reporter for QueueReporter {
    fn progress(&self, percent: f32, stage: &str) {
        self.sink
            .on_job_progress(&self.job_id, percent.clamp(0.0, 1.0), stage);
    }

    fn log(&self, level: LogLevel, message: &str) {
        tracing::debug!(job = %self.job_id, level = ?level, "{message}");
        self.sink.on_job_log(&self.job_id, level, message);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// 队列执行器。
#[derive(Debug, Clone)]
pub struct QueueRunner {
    concurrency: usize,
}

impl Default for QueueRunner {
    fn default() -> Self {
        QueueRunner {
            concurrency: default_concurrency(),
        }
    }
}

impl QueueRunner {
    /// 指定并发度；`0` 表示用默认值。
    pub fn new(concurrency: usize) -> Self {
        QueueRunner {
            concurrency: if concurrency == 0 {
                default_concurrency()
            } else {
                concurrency
            },
        }
    }

    /// 实际并发度。
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// 执行任务清单。
    ///
    /// * 输出目录会先做非破坏性校验。
    /// * 每个任务先写临时文件，成功后原子提交；失败/取消自动清理临时文件。
    /// * 取消后未开始的任务记为 `Cancelled`，已完成的文件保留。
    pub fn run(
        &self,
        plan: &JobPlan,
        pipeline: &dyn Pipeline,
        sink: Arc<dyn EventSink>,
        cancel: CancelToken,
    ) -> Result<JobReport> {
        plan.validate()?;
        let started = Instant::now();
        let total = plan.jobs.len();
        if total == 0 {
            return Ok(JobReport {
                output_dir: plan.output_dir.clone(),
                results: Vec::new(),
                elapsed_ms: 0,
                concurrency: 0,
            });
        }

        let resolver = Arc::new(OutputResolver::new(plan.output_dir.clone(), plan.policy)?);
        let queue = Arc::new(Mutex::new(VecDeque::from_iter(0..total)));
        let collected: Arc<Mutex<Vec<Option<JobResult>>>> =
            Arc::new(Mutex::new((0..total).map(|_| None).collect()));
        let workers = self.concurrency.clamp(1, total);

        std::thread::scope(|scope| {
            for _ in 0..workers {
                let queue = Arc::clone(&queue);
                let resolver = Arc::clone(&resolver);
                let collected = Arc::clone(&collected);
                let sink = Arc::clone(&sink);
                let cancel = cancel.clone();
                scope.spawn(move || loop {
                    let next = {
                        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                        q.pop_front()
                    };
                    let Some(index) = next else { break };
                    let job = &plan.jobs[index];
                    let result = run_one(job, &resolver, pipeline, &sink, &cancel);
                    sink.on_job_finished(&result);
                    let mut guard = collected.lock().unwrap_or_else(|e| e.into_inner());
                    guard[index] = Some(result);
                });
            }
        });

        let slots = match Arc::try_unwrap(collected) {
            Ok(mutex) => mutex.into_inner().unwrap_or_else(|e| e.into_inner()),
            Err(arc) => arc.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        };

        let results: Vec<JobResult> = slots
            .into_iter()
            .enumerate()
            .map(|(i, r)| r.unwrap_or_else(|| JobResult::cancelled(&plan.jobs[i])))
            .collect();

        Ok(JobReport {
            output_dir: plan.output_dir.clone(),
            results,
            elapsed_ms: started.elapsed().as_millis() as u64,
            concurrency: workers,
        })
    }
}

/// 两个路径是否指向同一个目标位置。
///
/// 先解析父目录（真实路径），再按文件名比较（Windows/macOS 大小写不敏感）。
/// 用于“输出 == 源”这种会丢数据的场景的硬保护。
fn same_path(a: &Path, b: &Path) -> bool {
    let parent = |p: &Path| {
        p.parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()))
    };
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_string());

    match (parent(a), parent(b), name(a), name(b)) {
        (Some(dir_a), Some(dir_b), Some(name_a), Some(name_b)) => {
            let same_dir = if cfg!(windows) {
                dir_a.to_string_lossy().eq_ignore_ascii_case(&dir_b.to_string_lossy())
            } else {
                dir_a == dir_b
            };
            let same_name = if cfg!(windows) {
                name_a.eq_ignore_ascii_case(&name_b)
            } else {
                name_a == name_b
            };
            same_dir && same_name
        }
        _ => a == b,
    }
}

fn run_one(
    job: &Job,
    resolver: &OutputResolver,
    pipeline: &dyn Pipeline,
    sink: &Arc<dyn EventSink>,
    cancel: &CancelToken,
) -> JobResult {
    if cancel.is_cancelled() {
        return JobResult::cancelled(job);
    }
    sink.on_job_started(job);
    let start = Instant::now();

    let decision = match resolver.resolve(&job.output_name) {
        Ok(d) => d,
        Err(e) => return JobResult::failed(job, &e, start.elapsed()),
    };

    let (temp, final_path) = match decision {
        OutputDecision::Skip { reason } => {
            sink.on_job_log(job.id.as_str(), LogLevel::Warn, &reason);
            return JobResult::skipped(job, reason);
        }
        OutputDecision::Write { temp, final_path } => (temp, final_path),
    };

    // 安全闸：无论如何不得把结果写回源文件本身
    // （否则“覆盖”策略下会先删除源再 rename → 源不可逆丢失）
    if same_path(&final_path, &job.input) {
        return JobResult::failed(
            job,
            &TfError::Input(format!(
                "输出路径与源文件相同（{}）——为保证非破坏性，已拒绝执行",
                job.input.display()
            )),
            start.elapsed(),
        );
    }

    let guard = TempGuard::new(temp);
    let reporter: Arc<dyn Reporter> = Arc::new(QueueReporter {
        job_id: job.id.clone(),
        sink: Arc::clone(sink),
        cancel: cancel.clone(),
    });
    let ctx = JobContext {
        job,
        output_dir: resolver.dir(),
        reporter,
    };

    match pipeline.produce(job, guard.path(), &ctx) {
        Ok(produced) => {
            if cancel.is_cancelled() {
                // 已取消：丢弃临时结果（guard 负责清理），保留源文件
                return JobResult::cancelled(job);
            }
            match guard.commit(&final_path) {
                Ok(path) => {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    sink.on_job_log(
                        job.id.as_str(),
                        LogLevel::Info,
                        &format!("已输出 {}", path.display()),
                    );
                    JobResult::success(job, path, bytes, start.elapsed()).with_note(produced.note)
                }
                Err(e) => JobResult::failed(job, &e, start.elapsed()),
            }
        }
        Err(e) if e.category() == tf_core::error::ErrorCategory::Cancelled => {
            let mut result = JobResult::cancelled(job);
            result.elapsed_ms = start.elapsed().as_millis() as u64;
            result
        }
        Err(e) => {
            sink.on_job_log(job.id.as_str(), LogLevel::Error, &e.to_string());
            JobResult::failed(job, &e, start.elapsed())
        }
    }
}

/// 等待一小段时间（测试用：模拟慢速流水线）。
#[cfg(test)]
pub(crate) fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// 报告里是否有失败项。
pub fn has_failures(report: &JobReport) -> bool {
    report
        .results
        .iter()
        .any(|r| r.status == TaskStatus::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{
        plan_convert, plan_rename, ChannelMode, ConvertConfig, ConvertSource, JobPayload,
        RenameEntry,
    };
    use crate::pipeline::ProduceReport;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    struct MockPipeline {
        active: AtomicUsize,
        peak: AtomicUsize,
        delay_ms: u64,
        fail_on: Option<String>,
        payload_bytes: usize,
        called: AtomicUsize,
    }

    impl MockPipeline {
        fn new(payload_bytes: usize, delay_ms: u64) -> Self {
            MockPipeline {
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                delay_ms,
                fail_on: None,
                payload_bytes,
                called: AtomicUsize::new(0),
            }
        }

        fn failing(payload_bytes: usize, on: &str) -> Self {
            MockPipeline {
                fail_on: Some(on.to_string()),
                ..MockPipeline::new(payload_bytes, 0)
            }
        }

        fn calls(&self) -> usize {
            self.called.load(Ordering::SeqCst)
        }
    }

    impl Pipeline for MockPipeline {
        fn produce(
            &self,
            job: &Job,
            target: &Path,
            ctx: &JobContext<'_>,
        ) -> Result<ProduceReport> {
            self.called.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            if self.delay_ms > 0 {
                sleep_ms(self.delay_ms);
            }
            let result = if self.fail_on.as_deref() == Some(job.output_name.as_str()) {
                Err(TfError::Decode(format!("模拟解码失败：{}", job.input_name())))
            } else {
                ctx.reporter.progress(0.5, "处理中");
                std::fs::write(target, vec![b'x'; self.payload_bytes])
                    .map_err(|e| TfError::Io(e.to_string()))?;
                ctx.reporter.progress(1.0, "完成");
                Ok(ProduceReport {
                    note: Some("模拟输出".into()),
                    ..ProduceReport::default()
                })
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }

    struct Setup {
        _root: tempfile::TempDir,
        in_dir: PathBuf,
        out_dir: PathBuf,
    }

    fn setup(extra: usize) -> (Setup, Vec<ConvertSource>) {
        let root = tempfile::tempdir().unwrap();
        let in_dir = root.path().join("in");
        let out_dir = root.path().join("out");
        std::fs::create_dir_all(&in_dir).unwrap();
        let mut sources = Vec::new();
        for i in 0..extra {
            let path = in_dir.join(format!("track{i}.flac"));
            std::fs::write(&path, b"source").unwrap();
            sources.push(ConvertSource {
                path: path.clone(),
                media: media(&path),
            });
        }
        (
            Setup {
                _root: root,
                in_dir,
                out_dir,
            },
            sources,
        )
    }

    fn media(path: &Path) -> tf_core::model::MediaInfo {
        tf_core::model::MediaInfo {
            path: path.to_path_buf(),
            format: Some(tf_core::model::AudioFormat::Flac),
            container: "flac".into(),
            codec: "flac".into(),
            sample_rate: Some(44_100),
            bits_per_sample: Some(16),
            is_float: false,
            channels: Some(2),
            channel_layout: Some("stereo".into()),
            duration_secs: Some(1.0),
            frames: Some(44_100),
            bit_rate: None,
            size_bytes: 6,
        }
    }

    fn temp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(".tf-tmp-"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn runs_all_jobs_and_commits_atomically() {
        let (setup, sources) = setup(3);
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        let pipeline = MockPipeline::new(64, 0);
        let sink = Arc::new(CollectingSink::new());
        let report = QueueRunner::new(2)
            .run(&plan, &pipeline, sink.clone(), CancelToken::new())
            .unwrap();

        let summary = report.summary();
        assert_eq!(summary.success, 3);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.bytes, 3 * 64);
        assert!(temp_files(&setup.out_dir).is_empty(), "不应残留临时文件");
        for i in 0..3 {
            assert!(setup.out_dir.join(format!("track{i}.flac")).exists());
        }
        assert_eq!(sink.finished_count(), 3);
        assert!(sink.progress_count() >= 6);
        assert!(sink.log_messages().iter().any(|m| m.contains("已输出")));
        assert_eq!(report.concurrency, 2);
        assert!(!has_failures(&report));
    }

    #[test]
    fn concurrency_is_bounded_by_runner_setting() {
        let (setup, sources) = setup(6);
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        let pipeline = Arc::new(MockPipeline::new(8, 25));
        let pipeline_ref: &MockPipeline = &pipeline;
        let report = QueueRunner::new(3)
            .run(
                &plan,
                pipeline_ref,
                Arc::new(NullSink),
                CancelToken::new(),
            )
            .unwrap();
        assert_eq!(report.summary().success, 6);
        assert!(pipeline.peak.load(Ordering::SeqCst) <= 3, "并发未受限");
        assert!(pipeline.peak.load(Ordering::SeqCst) >= 2, "并发未生效");
    }

    #[test]
    fn cancel_marks_remaining_jobs_and_keeps_sources() {
        let (setup, sources) = setup(4);
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        let pipeline = MockPipeline::new(8, 5);
        let token = CancelToken::new();
        let cancel_on_first = CancelOnFirstFinish {
            token: token.clone(),
        };
        let report = QueueRunner::new(1)
            .run(&plan, &pipeline, Arc::new(cancel_on_first), token)
            .unwrap();
        let summary = report.summary();
        assert_eq!(summary.total, 4);
        assert!(summary.cancelled >= 3, "{summary:?}");
        assert_eq!(summary.failed, 0);
        assert!(temp_files(&setup.out_dir).is_empty());
        // 源文件完好
        for i in 0..4 {
            assert!(setup.in_dir.join(format!("track{i}.flac")).exists());
        }
    }

    struct CancelOnFirstFinish {
        token: CancelToken,
    }

    impl EventSink for CancelOnFirstFinish {
        fn on_job_finished(&self, _result: &JobResult) {
            self.token.cancel();
        }
    }

    #[test]
    fn failures_are_recorded_with_category_and_cleaned_up() {
        let (setup, sources) = setup(2);
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        let pipeline = MockPipeline::failing(8, "track1.flac");
        let report = QueueRunner::new(1)
            .run(&plan, &pipeline, Arc::new(NullSink), CancelToken::new())
            .unwrap();
        let summary = report.summary();
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.success, 1);
        let failure = report.failures()[0];
        assert_eq!(failure.error_category, Some(tf_core::error::ErrorCategory::Decode));
        assert!(failure.error.as_deref().unwrap().contains("模拟解码失败"));
        assert_eq!(temp_files(&setup.out_dir).len(), 0);
    }

    #[test]
    fn skip_policy_does_not_invoke_pipeline() {
        let (setup, sources) = setup(1);
        std::fs::create_dir_all(&setup.out_dir).unwrap();
        std::fs::write(setup.out_dir.join("track0.flac"), b"existing").unwrap();
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        let pipeline = MockPipeline::new(8, 0);
        let report = QueueRunner::new(1)
            .run(&plan, &pipeline, Arc::new(NullSink), CancelToken::new())
            .unwrap();
        assert_eq!(report.summary().skipped, 1);
        assert_eq!(pipeline.calls(), 0);
        assert_eq!(std::fs::read(setup.out_dir.join("track0.flac")).unwrap(), b"existing");
    }

    #[test]
    fn overwrite_policy_replaces_existing_file() {
        let (setup, sources) = setup(1);
        std::fs::create_dir_all(&setup.out_dir).unwrap();
        std::fs::write(setup.out_dir.join("track0.flac"), b"old").unwrap();
        let plan = plan_convert(
            sources,
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Overwrite,
        )
        .unwrap();
        let report = QueueRunner::new(1)
            .run(&plan, &MockPipeline::new(16, 0), Arc::new(NullSink), CancelToken::new())
            .unwrap();
        assert_eq!(report.summary().success, 1);
        assert_eq!(std::fs::read(setup.out_dir.join("track0.flac")).unwrap().len(), 16);
    }

    #[test]
    fn rename_jobs_copy_without_touching_source() {
        let (setup, _sources) = setup(1);
        let original = setup.in_dir.join("track0.flac");
        let plan = plan_rename(
            vec![RenameEntry {
                source: original.clone(),
                new_name: "Artist - Title.flac".into(),
            }],
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        struct CopyPipeline;
        impl Pipeline for CopyPipeline {
            fn produce(&self, job: &Job, target: &Path, _ctx: &JobContext<'_>) -> Result<ProduceReport> {
                assert!(matches!(job.payload, JobPayload::Rename));
                std::fs::copy(&job.input, target)
                    .map_err(|e| TfError::Io(e.to_string()))?;
                Ok(ProduceReport::default())
            }
        }
        let report = QueueRunner::new(1)
            .run(&plan, &CopyPipeline, Arc::new(NullSink), CancelToken::new())
            .unwrap();
        assert_eq!(report.summary().success, 1);
        assert!(setup.out_dir.join("Artist - Title.flac").exists());
        assert!(original.exists(), "源文件必须保留");
    }

    #[test]
    fn output_dir_equal_to_source_dir_is_rejected_before_running() {
        let (setup, sources) = setup(1);
        let plan = plan_convert(
            sources,
            &ConvertConfig {
                channel_mode: ChannelMode::Keep,
                ..ConvertConfig::default()
            },
            &setup.in_dir,
            crate::output::ConflictPolicy::Skip,
        );
        // plan_convert 已经拦下；这里再验证 runner 的校验分支
        assert!(plan.is_err());
        let good = plan_convert(
            vec![ConvertSource {
                path: setup.in_dir.join("track0.flac"),
                media: media(&setup.in_dir.join("track0.flac")),
            }],
            &ConvertConfig::default(),
            &setup.out_dir,
            crate::output::ConflictPolicy::Skip,
        )
        .unwrap();
        assert!(QueueRunner::new(1)
            .run(&good, &MockPipeline::new(1, 0), Arc::new(NullSink), CancelToken::new())
            .is_ok());
    }

    #[test]
    fn empty_plan_produces_empty_report() {
        let (setup, _) = setup(0);
        let plan = JobPlan {
            output_dir: setup.out_dir.clone(),
            policy: crate::output::ConflictPolicy::Skip,
            jobs: Vec::new(),
        };
        let report = QueueRunner::new(2)
            .run(&plan, &MockPipeline::new(1, 0), Arc::new(NullSink), CancelToken::new())
            .unwrap();
        assert_eq!(report.summary().total, 0);
        assert_eq!(report.concurrency, 0);
    }

    #[test]
    fn cancel_token_helpers() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        assert!(token.check().is_ok());
        token.cancel();
        assert!(token.is_cancelled());
        assert_eq!(token.check().unwrap_err().category(), tf_core::error::ErrorCategory::Cancelled);
        let cloned = token.clone();
        assert!(cloned.is_cancelled());
    }

    #[test]
    fn default_concurrency_is_capped_at_four() {
        let n = default_concurrency();
        assert!((1..=4).contains(&n));
        assert_eq!(QueueRunner::new(0).concurrency(), n);
        assert_eq!(QueueRunner::new(7).concurrency(), 7);
        assert_eq!(JobPayload::Rename, JobPayload::Rename);
    }
}
