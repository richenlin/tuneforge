//! ffmpeg `-progress` 输出解析与子进程运行辅助（设计方案 §10.5）。
//!
//! ffmpeg 的 `-progress` 是 `key=value` 行；`out_time_us` / `out_time_ms` 都是**微秒**
//! （后者是历史命名，这里按微秒解释）。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::Stdio;

use serde::Serialize;

/// 处理阶段（UI 用来显示“正在解码/编码”）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    /// 探测。
    Probe,
    /// 解码。
    Decode,
    /// 处理（DSP）。
    Process,
    /// 编码。
    Encode,
    /// 写标签。
    Tag,
}

/// 单个文件的进度事件。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaProgress {
    /// 阶段。
    pub stage: ProgressStage,
    /// 完成百分比（0..=1），无总时长时为 `None`。
    pub percent: Option<f64>,
    /// 已处理时长（秒）。
    pub out_time_secs: Option<f64>,
    /// 处理速度（例如 32.5 表示 32.5x）。
    pub speed: Option<f64>,
    /// 是否已完成。
    pub finished: bool,
}

impl MediaProgress {
    /// 构造一个“开始了”的事件。
    pub fn start(stage: ProgressStage) -> Self {
        MediaProgress {
            stage,
            percent: None,
            out_time_secs: None,
            speed: None,
            finished: false,
        }
    }
}

/// 进度回调。
pub type ProgressCallback<'a> = dyn Fn(MediaProgress) + Send + Sync + 'a;

/// ffmpeg `-progress` 行的累加器。
#[derive(Debug, Clone, Default)]
pub struct ProgressAcc {
    /// 已处理时长（秒）。
    pub out_time_secs: Option<f64>,
    /// 速度。
    pub speed: Option<f64>,
    /// 是否收到 `progress=end`。
    pub finished: bool,
    /// stderr 尾部（错误诊断）。
    pub stderr_tail: VecDeque<String>,
}

impl ProgressAcc {
    /// 记录一行输出；返回 **true** 表示这是一条新的进度数据。
    pub fn feed(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            self.push_stderr(trimmed);
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "out_time_us" => {
                self.out_time_secs = value.parse::<f64>().ok().map(|us| us / 1_000_000.0);
                true
            }
            "out_time_ms" => {
                // ffmpeg 的 out_time_ms 实际是微秒
                if self.out_time_secs.is_none() {
                    self.out_time_secs = value.parse::<f64>().ok().map(|us| us / 1_000_000.0);
                }
                true
            }
            "out_time" => {
                if self.out_time_secs.is_none() {
                    self.out_time_secs = parse_hms(value);
                }
                true
            }
            "speed" => {
                let v = value.trim_end_matches('x').trim();
                self.speed = v.parse::<f64>().ok();
                true
            }
            "progress" => {
                if value == "end" {
                    self.finished = true;
                }
                true
            }
            _ => false,
        }
    }

    fn push_stderr(&mut self, line: &str) {
        if self.stderr_tail.len() >= 16 {
            self.stderr_tail.pop_front();
        }
        self.stderr_tail.push_back(line.to_string());
    }

    /// 生成进度事件。
    pub fn event(&self, stage: ProgressStage, duration_secs: Option<f64>) -> MediaProgress {
        let percent = match (self.out_time_secs, duration_secs) {
            (Some(t), Some(d)) if d > 0.0 => Some((t / d).clamp(0.0, 1.0)),
            _ => None,
        };
        MediaProgress {
            stage,
            percent,
            out_time_secs: self.out_time_secs,
            speed: self.speed,
            finished: self.finished,
        }
    }

    /// stderr 尾部拼成的错误信息（用于错误分类）。
    pub fn stderr_text(&self) -> String {
        self.stderr_tail
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 解析 `HH:MM:SS.micro`。
pub fn parse_hms(text: &str) -> Option<f64> {
    let mut parts = text.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// 在后台读取子进程 stderr，喂给进度累加器并回调。
pub struct StderrPump {
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StderrPump {
    /// 启动读取线程。`acc` 为共享累加器（`Arc<Mutex<_>>`）。
    pub fn spawn(
        stderr: std::process::ChildStderr,
        acc: std::sync::Arc<std::sync::Mutex<ProgressAcc>>,
        stage: ProgressStage,
        duration_secs: Option<f64>,
        callback: Option<std::sync::Arc<ProgressCallback<'static>>>,
    ) -> Self {
        let handle = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let mut guard = match acc.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let is_progress = guard.feed(&line);
                if is_progress {
                    if let Some(cb) = &callback {
                        let event = guard.event(stage, duration_secs);
                        cb(event);
                    }
                }
            }
        });
        StderrPump {
            handle: Some(handle),
        }
    }

    /// 等待读取线程结束。
    pub fn join(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 子进程通用配置：不读 stdin、隐藏 banner、错误级别日志、`-progress` 到 stderr。
pub fn common_io_args() -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:2".into(),
    ]
}

/// 统一的子进程构造（stdin/stdout/stderr 均由调用方决定）。
pub fn build_command(program: &std::path::Path, args: &[String]) -> std::process::Command {
    let mut cmd = crate::process::command(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_progress_keys() {
        let mut acc = ProgressAcc::default();
        assert!(acc.feed("out_time_us=1500000"));
        assert!(acc.feed("speed=32.5x"));
        assert!(acc.feed("progress=continue"));
        assert!(!acc.finished);
        assert_eq!(acc.out_time_secs, Some(1.5));
        assert_eq!(acc.speed, Some(32.5));
        assert!(acc.feed("progress=end"));
        assert!(acc.finished);
    }

    #[test]
    fn out_time_ms_is_interpreted_as_microseconds() {
        let mut acc = ProgressAcc::default();
        acc.feed("out_time_ms=2500000");
        assert_eq!(acc.out_time_secs, Some(2.5));
    }

    #[test]
    fn out_time_hms_fallback_works() {
        let mut acc = ProgressAcc::default();
        acc.feed("out_time=00:01:30.500000");
        assert_eq!(acc.out_time_secs, Some(90.5));
        assert_eq!(parse_hms("01:00:00.000000"), Some(3600.0));
        assert_eq!(parse_hms("bad"), None);
    }

    #[test]
    fn percent_is_clamped_and_none_without_duration() {
        let mut acc = ProgressAcc::default();
        acc.feed("out_time_us=30000000");
        let e = acc.event(ProgressStage::Encode, Some(10.0));
        assert_eq!(e.percent, Some(1.0));
        assert_eq!(e.stage, ProgressStage::Encode);
        let e = acc.event(ProgressStage::Encode, None);
        assert_eq!(e.percent, None);
    }

    #[test]
    fn stderr_lines_are_kept_for_diagnostics() {
        let mut acc = ProgressAcc::default();
        acc.feed("some random error text");
        acc.feed("[flac @ 0x1] invalid something");
        for i in 0..40 {
            acc.feed(&format!("line {i}"));
        }
        assert!(acc.stderr_tail.len() <= 16);
        assert!(acc.stderr_text().contains("line 39"));
    }

    #[test]
    fn common_args_include_progress_pipe() {
        let args = common_io_args();
        assert!(args.contains(&"-progress".to_string()));
        assert!(args.contains(&"pipe:2".to_string()));
    }

    #[test]
    fn start_event_is_not_finished() {
        let e = MediaProgress::start(ProgressStage::Decode);
        assert!(!e.finished);
        assert_eq!(e.percent, None);
    }
}
