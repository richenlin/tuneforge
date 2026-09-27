//! 结果统计与导出（设计方案 §12）。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tf_core::error::ErrorCategory;

use crate::plan::{Job, JobKind};

/// 单个文件任务的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 成功。
    Success,
    /// 按冲突策略跳过。
    Skipped,
    /// 失败。
    Failed,
    /// 因取消而中止。
    Cancelled,
}

impl TaskStatus {
    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Success => "成功",
            TaskStatus::Skipped => "跳过",
            TaskStatus::Failed => "失败",
            TaskStatus::Cancelled => "已取消",
        }
    }
}

/// 单个文件任务的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    /// 任务 id。
    pub job_id: String,
    /// 任务类型。
    pub kind: JobKind,
    /// 源文件。
    pub input: PathBuf,
    /// 源文件名。
    pub input_name: String,
    /// 输出文件（成功时）。
    pub output: Option<PathBuf>,
    /// 状态。
    pub status: TaskStatus,
    /// 错误信息。
    pub error: Option<String>,
    /// 错误分类。
    pub error_category: Option<ErrorCategory>,
    /// 附加说明（例如“已限幅 -3.2 dB”）。
    pub note: Option<String>,
    /// 输出字节数。
    pub bytes: u64,
    /// 耗时（毫秒）。
    pub elapsed_ms: u64,
}

impl JobResult {
    /// 构造成功结果。
    pub fn success(job: &Job, output: PathBuf, bytes: u64, elapsed: Duration) -> Self {
        JobResult {
            job_id: job.id.clone(),
            kind: job.kind,
            input: job.input.clone(),
            input_name: job.input_name(),
            output: Some(output),
            status: TaskStatus::Success,
            error: None,
            error_category: None,
            note: None,
            bytes,
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }

    /// 构造跳过结果。
    pub fn skipped(job: &Job, reason: String) -> Self {
        JobResult {
            job_id: job.id.clone(),
            kind: job.kind,
            input: job.input.clone(),
            input_name: job.input_name(),
            output: None,
            status: TaskStatus::Skipped,
            error: None,
            error_category: None,
            note: Some(reason),
            bytes: 0,
            elapsed_ms: 0,
        }
    }

    /// 构造取消结果。
    pub fn cancelled(job: &Job) -> Self {
        JobResult {
            job_id: job.id.clone(),
            kind: job.kind,
            input: job.input.clone(),
            input_name: job.input_name(),
            output: None,
            status: TaskStatus::Cancelled,
            error: Some("任务已取消".into()),
            error_category: Some(ErrorCategory::Cancelled),
            note: None,
            bytes: 0,
            elapsed_ms: 0,
        }
    }

    /// 构造失败结果。
    pub fn failed(job: &Job, error: &tf_core::TfError, elapsed: Duration) -> Self {
        JobResult {
            job_id: job.id.clone(),
            kind: job.kind,
            input: job.input.clone(),
            input_name: job.input_name(),
            output: None,
            status: TaskStatus::Failed,
            error: Some(error.to_string()),
            error_category: Some(error.category()),
            note: None,
            bytes: 0,
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }

    /// 是否成功。
    pub fn is_success(&self) -> bool {
        self.status == TaskStatus::Success
    }

    /// 补充说明。
    pub fn with_note(mut self, note: Option<String>) -> Self {
        if note.is_some() {
            self.note = note;
        }
        self
    }
}

/// 汇总统计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// 任务总数。
    pub total: usize,
    /// 成功数。
    pub success: usize,
    /// 跳过数。
    pub skipped: usize,
    /// 失败数。
    pub failed: usize,
    /// 取消数。
    pub cancelled: usize,
    /// 输出总字节数。
    pub bytes: u64,
    /// 总耗时（毫秒）。
    pub elapsed_ms: u64,
}

impl Summary {
    /// 完成率（0..=1）。
    pub fn completion(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.success + self.skipped) as f64 / self.total as f64
        }
    }

    /// 中文摘要文本。
    pub fn text(&self) -> String {
        format!(
            "共 {} 个任务：成功 {}，跳过 {}，失败 {}，取消 {}（{:.1} s）",
            self.total,
            self.success,
            self.skipped,
            self.failed,
            self.cancelled,
            self.elapsed_ms as f64 / 1000.0
        )
    }
}

/// 一次运行的完整报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobReport {
    /// 输出目录。
    pub output_dir: PathBuf,
    /// 每个文件的结果（按任务顺序）。
    pub results: Vec<JobResult>,
    /// 总耗时（毫秒）。
    pub elapsed_ms: u64,
    /// 实际使用的并发度。
    pub concurrency: usize,
}

impl JobReport {
    /// 汇总。
    pub fn summary(&self) -> Summary {
        let mut s = Summary {
            total: self.results.len(),
            elapsed_ms: self.elapsed_ms,
            ..Summary::default()
        };
        for r in &self.results {
            match r.status {
                TaskStatus::Success => {
                    s.success += 1;
                    s.bytes += r.bytes;
                }
                TaskStatus::Skipped => s.skipped += 1,
                TaskStatus::Failed => s.failed += 1,
                TaskStatus::Cancelled => s.cancelled += 1,
            }
        }
        s
    }

    /// 失败项（用于“仅重跑失败项”）。
    pub fn failures(&self) -> Vec<&JobResult> {
        self.results
            .iter()
            .filter(|r| r.status == TaskStatus::Failed)
            .collect()
    }

    /// 导出 JSON。
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// 导出 CSV（含表头，RFC4180 转义）。
    pub fn to_csv(&self) -> String {
        let mut out = String::from(
            "status,kind,input,output,error_category,error,note,bytes,elapsed_ms\n",
        );
        for r in &self.results {
            let row = [
                r.status.label().to_string(),
                serde_json::to_string(&r.kind)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
                r.input.display().to_string(),
                r.output
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                r.error_category
                    .map(|c| c.label().to_string())
                    .unwrap_or_default(),
                r.error.clone().unwrap_or_default(),
                r.note.clone().unwrap_or_default(),
                r.bytes.to_string(),
                r.elapsed_ms.to_string(),
            ];
            out.push_str(&row.map(|v| csv_field(&v)).join(","));
            out.push('\n');
        }
        out
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tf_core::error::TfError;

    fn job(kind: JobKind, input: &str) -> Job {
        crate::plan::test_job(kind, input)
    }

    #[test]
    fn summary_counts_every_status() {
        let j = job(JobKind::Convert, "C:/in/a.flac");
        let report = JobReport {
            output_dir: PathBuf::from("C:/out"),
            results: vec![
                JobResult::success(&j, PathBuf::from("C:/out/a.flac"), 1024, Duration::from_millis(50))
                    .with_note(Some("已限幅 -3.0 dB".into())),
                JobResult::skipped(&j, "已存在".into()),
                JobResult::failed(&j, &TfError::Decode("boom".into()), Duration::from_millis(10)),
                JobResult::cancelled(&j),
            ],
            elapsed_ms: 1234,
            concurrency: 4,
        };
        let s = report.summary();
        assert_eq!(s.total, 4);
        assert_eq!(s.success, 1);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.cancelled, 1);
        assert_eq!(s.bytes, 1024);
        assert!((s.completion() - 0.5).abs() < 1e-9);
        assert!(s.text().contains("成功 1"));
        assert_eq!(report.failures().len(), 1);
    }

    #[test]
    fn csv_is_escaped_and_has_header() {
        let j = job(JobKind::Tags, "C:/in/a, b.flac");
        let mut result = JobResult::success(&j, PathBuf::from("C:/out/a.flac"), 1, Duration::from_millis(1));
        result.note = Some("含 \"引号\" 与,逗号".into());
        let report = JobReport {
            output_dir: PathBuf::from("C:/out"),
            results: vec![result],
            elapsed_ms: 5,
            concurrency: 1,
        };
        let csv = report.to_csv();
        let mut lines = csv.lines();
        assert!(lines.next().unwrap().starts_with("status,kind,input"));
        let row = lines.next().unwrap();
        assert!(row.contains("\"C:/in/a, b.flac\""));
        assert!(row.contains("\"\"引号\"\""));
        assert_eq!(lines.count(), 0);
    }

    #[test]
    fn json_export_roundtrips() {
        let j = job(JobKind::Normalize, "C:/in/a.flac");
        let report = JobReport {
            output_dir: PathBuf::from("C:/out"),
            results: vec![JobResult::success(
                &j,
                PathBuf::from("C:/out/a.flac"),
                10,
                Duration::from_millis(3),
            )],
            elapsed_ms: 3,
            concurrency: 2,
        };
        let json = report.to_json();
        let back: JobReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn status_labels_are_chinese() {
        assert_eq!(TaskStatus::Success.label(), "成功");
        assert_eq!(TaskStatus::Cancelled.label(), "已取消");
    }
}
