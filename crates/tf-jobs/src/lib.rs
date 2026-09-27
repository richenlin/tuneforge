//! 任务队列与输出（设计方案 §10）。
//!
//! * 非破坏性：只读源文件，结果写入用户指定目录（`plan` 会拒绝输出目录 = 源目录）。
//! * 先写临时文件（`.tf-tmp-<uuid>`），成功后再原子 `rename`；失败/取消时删除临时文件。
//! * 冲突策略：跳过（默认）/ 覆盖 / 自动重命名 ` (1)`。
//! * 并发：默认 `min(CPU 核心数, 4)`（FFmpeg 自身也可能多线程）。
//! * 可取消：每个文件独立任务，已完成的文件保留；任务清单可导出 CSV/JSON。
#![forbid(unsafe_code)]

pub mod output;
pub mod pipeline;
pub mod plan;
pub mod queue;
pub mod report;

pub use output::{
    temp_path, validate_output_dir, ConflictPolicy, OutputDecision, OutputResolver, TempGuard,
};
pub use pipeline::{FfmpegPipeline, Pipeline, ProduceReport};
pub use plan::{
    plan_convert, plan_normalize, plan_rename, plan_tags, ChannelMode, ConvertConfig,
    ConvertPayload, ConvertSource, CoverAction, Job, JobKind, JobPayload, JobPlan,
    NormalizeConfig, NormalizePayload, RenameEntry, TagsEntry, TagsPayload,
};
pub use queue::{
    default_concurrency, has_failures, CancelToken, CollectingSink, EventSink, FnSink, JobContext,
    LogLevel, NullSink, QueueEvent, QueueRunner, Reporter,
};
pub use report::{JobReport, JobResult, Summary, TaskStatus};
