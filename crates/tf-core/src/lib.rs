//! Tuneforge 领域核心库。
//!
//! 本 crate 只包含纯逻辑（模型、DSP、命名模板），**不依赖任何外部 I/O**，
//! 因此可以用合成信号做确定性单元测试。参见设计方案 §4.2 依赖方向：
//!
//! ```text
//! ui → src-tauri → tf-jobs → { tf-media, tf-tags } → tf-core
//! ```
#![forbid(unsafe_code)]

pub mod dither;
pub mod downmix;
pub mod error;
pub mod limiter;
pub mod loudness;
pub mod model;
pub mod naming;
pub mod parallel;
pub mod truepeak;
pub mod util;

pub use error::{ErrorCategory, Result, TfError};
pub use model::{
    AudioBuffer, AudioFormat, CoverArt, MediaInfo, MediaItem, SourceRating, TagField, Tags,
};
