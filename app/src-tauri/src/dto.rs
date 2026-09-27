//! 前后端 DTO（契约见 `docs/contracts/tauri-commands.md`）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tf_core::limiter::NormalizeParams;
use tf_core::model::{AudioFormat, TagField, Tags};
use tf_core::naming::{MissingFieldPolicy, SanitizeOptions};
use tf_jobs::{ConflictPolicy, ConvertConfig};

/// ffmpeg 定位状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    /// 是否可用。
    pub available: bool,
    /// 是否仍在后台探测（前端据此显示“检测中”）。
    pub probing: bool,
    /// ffmpeg 路径。
    pub ffmpeg_path: Option<String>,
    /// ffprobe 路径。
    pub ffprobe_path: Option<String>,
    /// 来源。
    pub source: Option<String>,
    /// 版本第一行。
    pub version: Option<String>,
    /// 可编码的输出格式 id。
    pub encodable: Vec<String>,
    /// 不可用格式及原因。
    pub unsupported: Vec<UnsupportedFormat>,
    /// 失败提示（中文）。
    pub message: Option<String>,
    /// 探测耗时（毫秒；未就绪时为 `None`）。
    pub probe_ms: Option<u64>,
    /// 是否命中磁盘缓存（命中则未启动子进程）。
    pub cached: bool,
}

impl FfmpegStatus {
    /// 由探测状态快照构造 DTO（命令层只读，不做任何子进程调用）。
    pub fn from_probe(state: &crate::state::ProbeState) -> Self {
        use crate::state::ProbeState;
        match state {
            ProbeState::Ready {
                paths,
                caps,
                cached,
                elapsed_ms,
            } => Self {
                available: true,
                probing: false,
                ffmpeg_path: Some(paths.ffmpeg.display().to_string()),
                ffprobe_path: Some(paths.ffprobe.display().to_string()),
                source: Some(paths.source.label().to_string()),
                version: Some(caps.version_line.clone()),
                encodable: caps
                    .encodable_formats()
                    .iter()
                    .map(|f| f.id().to_string())
                    .collect(),
                unsupported: caps
                    .unsupported_formats()
                    .into_iter()
                    .map(|(f, reason)| UnsupportedFormat {
                        format: f.id().to_string(),
                        label: f.label().to_string(),
                        reason: if f.can_encode() {
                            format!("当前 FFmpeg 缺少编码器 {reason}")
                        } else {
                            reason.to_string()
                        },
                    })
                    .collect(),
                message: None,
                probe_ms: Some(*elapsed_ms),
                cached: *cached,
            },
            ProbeState::Idle | ProbeState::Probing => Self {
                available: false,
                probing: true,
                ffmpeg_path: None,
                ffprobe_path: None,
                source: None,
                version: None,
                encodable: Vec::new(),
                unsupported: Vec::new(),
                message: Some("正在检测 FFmpeg…".to_string()),
                probe_ms: None,
                cached: false,
            },
            ProbeState::Failed(message) => Self {
                available: false,
                probing: false,
                ffmpeg_path: None,
                ffprobe_path: None,
                source: None,
                version: None,
                encodable: Vec::new(),
                unsupported: Vec::new(),
                message: Some(message.clone()),
                probe_ms: None,
                cached: false,
            },
        }
    }
}

/// 不可用格式。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedFormat {
    /// 格式 id。
    pub format: String,
    /// 展示名。
    pub label: String,
    /// 原因。
    pub reason: String,
}

/// 转换页格式下拉项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatOption {
    /// 格式 id。
    pub format: String,
    /// 展示名。
    pub label: String,
    /// 扩展名。
    pub extension: String,
    /// 是否无损。
    pub lossless: bool,
    /// 当前 ffmpeg 是否支持。
    pub available: bool,
    /// 不可用原因。
    pub reason: Option<String>,
    /// 推荐位深。
    pub default_bit_depth: Option<u16>,
    /// 推荐质量摘要（中文）。
    pub default_quality: String,
}

/// 扫描/探测进度（`scan:progress`），多文件拖入时前端据此锁定界面并显示进度。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// 已探测完成的文件数。
    pub done: usize,
    /// 本轮待探测的文件总数。
    pub total: usize,
}

/// 响度测量行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRow {
    /// 条目 id。
    pub id: String,
    /// 文件名。
    pub file_name: String,
    /// 集成响度（LUFS）。
    pub loudness_lufs: Option<f64>,
    /// 真峰值（dBTP）。
    pub true_peak_dbtp: Option<f64>,
    /// 样本峰值（dBFS）。
    pub sample_peak_dbfs: Option<f64>,
    /// 是否为假多声道。
    pub fake_multichannel: bool,
    /// 假多声道说明。
    pub note: Option<String>,
    /// 失败信息。
    pub error: Option<String>,
}

/// 单个标签字段编辑。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldEdit {
    /// 字段。
    pub field: TagField,
    /// 新值（`None` 表示清除）。
    pub value: Option<String>,
}

/// 转换页配置（前端传入）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertConfigDto {
    /// 编码参数。
    pub spec: tf_media::EncodeSpec,
    /// 保留标签。
    pub keep_tags: bool,
    /// 无损增益（dB）。
    pub gain_db: Option<f64>,
    /// 真峰值上限（dBTP）。
    pub ceiling_dbtp: Option<f64>,
    /// 声道模式。
    pub channel_mode: tf_jobs::ChannelMode,
    /// 假多声道折混。
    pub downmix_fake_multichannel: bool,
}

impl From<ConvertConfigDto> for ConvertConfig {
    fn from(value: ConvertConfigDto) -> Self {
        ConvertConfig {
            spec: value.spec,
            keep_tags: value.keep_tags,
            gain_db: value.gain_db,
            ceiling_dbtp: value.ceiling_dbtp,
            channel_mode: value.channel_mode,
            downmix_fake_multichannel: value.downmix_fake_multichannel,
        }
    }
}

/// 一次任务请求。
///
/// `rename_all_fields = "camelCase"`：前端按契约（`docs/contracts/tauri-commands.md`）
/// 发送 camelCase 字段名（如 `outputDir`），Tauri 只会对命令的**顶层参数名**做
/// camelCase → snake_case 映射，嵌套结构体由 serde 负责。
#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "page",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum JobRequest {
    /// 转换页。
    Convert {
        /// 输出目录。
        output_dir: String,
        /// 冲突策略。
        policy: ConflictPolicy,
        /// 选中的条目 id（空表示全部）。
        ids: Vec<String>,
        /// 配置。
        config: ConvertConfigDto,
    },
    /// 重命名页。
    Rename {
        /// 输出目录。
        output_dir: String,
        /// 冲突策略。
        policy: ConflictPolicy,
        /// 模板。
        template: String,
        /// 缺失字段策略。
        missing: MissingFieldPolicy,
        /// 合法化选项。
        sanitize: SanitizeOptions,
        /// 选中的条目 id（空表示全部）。
        ids: Vec<String>,
    },
    /// 归一化页。
    Normalize {
        /// 输出目录。
        output_dir: String,
        /// 冲突策略。
        policy: ConflictPolicy,
        /// 选中的条目 id（空表示全部）。
        ids: Vec<String>,
        /// 归一化参数。
        params: NormalizeParams,
        /// 目标格式。
        format: AudioFormat,
        /// 目标位深。
        bit_depth: Option<u16>,
        /// 保留标签。
        keep_tags: bool,
    },
    /// 标签页。
    Tags {
        /// 输出目录。
        output_dir: String,
        /// 冲突策略。
        policy: ConflictPolicy,
        /// 选中的条目 id（空表示全部）。
        ids: Vec<String>,
    },
}

impl JobRequest {
    /// 输出目录。
    pub fn output_dir(&self) -> &str {
        match self {
            JobRequest::Convert { output_dir, .. }
            | JobRequest::Rename { output_dir, .. }
            | JobRequest::Normalize { output_dir, .. }
            | JobRequest::Tags { output_dir, .. } => output_dir,
        }
    }

    /// 冲突策略。
    pub fn policy(&self) -> ConflictPolicy {
        match self {
            JobRequest::Convert { policy, .. }
            | JobRequest::Rename { policy, .. }
            | JobRequest::Normalize { policy, .. }
            | JobRequest::Tags { policy, .. } => *policy,
        }
    }

    /// 选中的条目 id。
    pub fn ids(&self) -> &[String] {
        match self {
            JobRequest::Convert { ids, .. }
            | JobRequest::Rename { ids, .. }
            | JobRequest::Normalize { ids, .. }
            | JobRequest::Tags { ids, .. } => ids,
        }
    }
}

/// 扫描/列表用的条目 id 生成。
pub fn item_id(path: &PathBuf) -> String {
    // 稳定 id：路径的哈希 + 规范字符串
    let text = path.to_string_lossy().to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 把错误转成前端可解析的字符串（JSON 形状）。
pub fn err_json(error: &tf_core::TfError) -> String {
    serde_json::json!({
        "message": error.to_string(),
        "category": error.category(),
        "categoryLabel": error.category().label(),
    })
    .to_string()
}

/// 标签为空时视为无值（前端传 `""` 也会被清理）。
pub fn normalize_tags(tags: &mut Tags) {
    let owned = std::mem::take(tags);
    *tags = owned.normalized();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：前端按契约发送 camelCase 字段名，必须能反序列化。
    #[test]
    fn job_request_accepts_camel_case_fields() {
        let json = serde_json::json!({
            "page": "convert",
            "outputDir": "D:/out",
            "policy": "skip",
            "ids": ["a"],
            "config": {
                "spec": {
                    "format": "flac",
                    "bitDepth": 24,
                    "sampleRate": null,
                    "channels": null,
                    "quality": { "kind": "flac_level", "level": 8 }
                },
                "keepTags": true,
                "gainDb": null,
                "ceilingDbtp": null,
                "channelMode": "keep",
                "downmixFakeMultichannel": true
            }
        });
        let request: JobRequest = serde_json::from_value(json).expect("camelCase 请求应可解析");
        assert_eq!(request.output_dir(), "D:/out");
        assert_eq!(request.ids().len(), 1);
        assert_eq!(request.ids()[0], "a");
        assert!(matches!(request, JobRequest::Convert { .. }));
    }

    #[test]
    fn normalize_request_accepts_camel_case_params() {
        let json = serde_json::json!({
            "page": "normalize",
            "outputDir": "D:/out",
            "policy": "overwrite",
            "ids": [],
            "params": {
                "targetLufs": -14.0,
                "ceilingDbtp": -1.0,
                "lookaheadMs": 3.0,
                "attackMs": 5.0,
                "releaseMs": 120.0,
                "downmixFakeMultichannel": true
            },
            "format": "flac",
            "bitDepth": 24,
            "keepTags": true
        });
        let request: JobRequest =
            serde_json::from_value(json).expect("camelCase 归一化请求应可解析");
        match request {
            JobRequest::Normalize { params, .. } => assert_eq!(params.target_lufs, -14.0),
            other => panic!("期望 Normalize，得到 {other:?}"),
        }
    }

    #[test]
    fn rename_request_accepts_camel_case_sanitize() {
        let json = serde_json::json!({
            "page": "rename",
            "outputDir": "D:/out",
            "policy": "rename",
            "template": "{artist} - {title}",
            "missing": "placeholder",
            "sanitize": {
                "replacement": "_",
                "collapseSpaces": true,
                "fullwidthToHalfwidth": true,
                "maxLen": 120,
                "fallback": "未命名"
            },
            "ids": []
        });
        let request: JobRequest =
            serde_json::from_value(json).expect("camelCase 重命名请求应可解析");
        match request {
            JobRequest::Rename { sanitize, .. } => {
                assert!(sanitize.collapse_spaces);
                assert_eq!(sanitize.max_len, 120);
            }
            other => panic!("期望 Rename，得到 {other:?}"),
        }
    }
}
