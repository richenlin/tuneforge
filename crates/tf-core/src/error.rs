//! 错误模型（设计方案 §12）。
//!
//! 每个错误都带一个稳定的分类，便于 UI 归类展示与结果汇总。

use serde::{Deserialize, Serialize};

/// 错误分类，与设计方案 §12 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// 输入错误（路径不存在、参数非法等）。
    Input,
    /// 不支持的格式或功能（如 APE 不能作为输出、缺少编码器）。
    Unsupported,
    /// 探测失败（ffprobe）。
    Probe,
    /// 解码失败（ffmpeg）。
    Decode,
    /// 编码失败（ffmpeg）。
    Encode,
    /// 标签读写失败。
    Tag,
    /// 磁盘 / 权限 / 文件系统。
    Io,
    /// 用户取消。
    Cancelled,
    /// 内部错误（不应发生）。
    Internal,
}

impl ErrorCategory {
    /// 中文标签，直接用于 UI。
    pub fn label(self) -> &'static str {
        match self {
            ErrorCategory::Input => "输入错误",
            ErrorCategory::Unsupported => "不支持的格式",
            ErrorCategory::Probe => "探测失败",
            ErrorCategory::Decode => "解码失败",
            ErrorCategory::Encode => "编码失败",
            ErrorCategory::Tag => "标签失败",
            ErrorCategory::Io => "磁盘/权限",
            ErrorCategory::Cancelled => "已取消",
            ErrorCategory::Internal => "内部错误",
        }
    }
}

/// Tuneforge 统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum TfError {
    #[error("输入错误：{0}")]
    Input(String),

    #[error("不支持的格式：{0}")]
    Unsupported(String),

    #[error("探测失败：{0}")]
    Probe(String),

    #[error("解码失败：{0}")]
    Decode(String),

    #[error("编码失败：{0}")]
    Encode(String),

    #[error("标签操作失败：{0}")]
    Tag(String),

    #[error("文件系统错误：{0}")]
    Io(String),

    #[error("任务已取消")]
    Cancelled,

    #[error("内部错误：{0}")]
    Internal(String),
}

impl TfError {
    /// 该错误所属分类。
    pub fn category(&self) -> ErrorCategory {
        match self {
            TfError::Input(_) => ErrorCategory::Input,
            TfError::Unsupported(_) => ErrorCategory::Unsupported,
            TfError::Probe(_) => ErrorCategory::Probe,
            TfError::Decode(_) => ErrorCategory::Decode,
            TfError::Encode(_) => ErrorCategory::Encode,
            TfError::Tag(_) => ErrorCategory::Tag,
            TfError::Io(_) => ErrorCategory::Io,
            TfError::Cancelled => ErrorCategory::Cancelled,
            TfError::Internal(_) => ErrorCategory::Internal,
        }
    }

    /// 该错误是否值得重试（保留给任务队列使用）。
    pub fn is_retryable(&self) -> bool {
        matches!(self, TfError::Io(_))
    }
}

impl From<std::io::Error> for TfError {
    fn from(value: std::io::Error) -> Self {
        TfError::Io(value.to_string())
    }
}

/// 便捷 `Result` 别名。
pub type Result<T> = std::result::Result<T, TfError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_is_retryable_and_classified() {
        let err = TfError::from(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(err.category(), ErrorCategory::Io);
        assert!(err.is_retryable());
        assert!(err.to_string().contains("denied"));
    }

    #[test]
    fn category_labels_are_chinese() {
        assert_eq!(ErrorCategory::Decode.label(), "解码失败");
        assert!(!TfError::Cancelled.is_retryable());
    }
}
