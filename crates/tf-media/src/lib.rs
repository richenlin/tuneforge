//! FFmpeg / FFprobe 子进程封装（设计方案 §4.1 `tf-media`）。
//!
//! 本 crate 只负责“调用外部工具并解析结果”，不做业务编排：
//! * [`locate`] 定位并校验捆绑的 ffmpeg / ffprobe
//! * [`capabilities`] 查询可用编码器（决定 UI 里哪些输出格式可用）
//! * [`probe`] 探测媒体信息
//! * [`decode`] 解码为 `f64` 平面 PCM
//! * [`encode`] 编码（含各格式推荐默认参数）
//! * [`scan`] 输入扫描
#![forbid(unsafe_code)]

pub mod capabilities;
pub mod decode;
pub mod encode;
pub mod locate;
pub mod probe;
pub mod process;
pub mod progress;
pub mod scan;

pub use capabilities::Capabilities;
pub use decode::{decode_to_buffer, DecodeOptions};
pub use encode::{
    can_stream_copy, default_spec, encode_buffer, format_rate, has_soxr, is_sample_rate_supported,
    recommended_sample_rate, resample_args, sample_rate_candidates, sample_rate_limit,
    sample_rate_options, stream_copy_args, transcode, transcode_copy, EncodeQuality, EncodeSpec,
    SampleRateOption,
};
pub use locate::{discover, FfmpegPaths, LocateSource};
pub use probe::{probe_file, read_media_info};
pub use progress::{MediaProgress, ProgressAcc};
pub use scan::{is_supported_audio, scan_inputs};
