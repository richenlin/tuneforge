//! 任务清单构建（设计方案 §4.3 的“生成任务列表”）。
//!
//! 每个功能页都有自己的 `plan_*`：把“已确认的预览/配置”变成可执行任务。
//! 预览与冲突检测在 UI 层（`tf-core::naming`）完成，这里只做任务装配与输出名。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tf_core::error::{Result, TfError};
use tf_core::model::{CoverArt, MediaInfo, Tags};
use tf_media::EncodeSpec;

use crate::output::{validate_output_dir, ConflictPolicy};

/// 功能页 / 任务类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// 格式转换。
    Convert,
    /// 文件名标准化（只改名，不转码）。
    Rename,
    /// 音量归一化。
    Normalize,
    /// 标签修改。
    Tags,
}

impl JobKind {
    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Convert => "格式转换",
            JobKind::Rename => "文件名标准化",
            JobKind::Normalize => "音量归一化",
            JobKind::Tags => "标签修改",
        }
    }
}

/// 声道处理方式（转换页高级选项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// 保持源声道。
    #[default]
    Keep,
    /// 折混为立体声。
    Stereo,
    /// 折混为单声道。
    Mono,
}

/// 转换页配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertConfig {
    /// 目标格式与编码参数。
    pub spec: EncodeSpec,
    /// 是否保留标签。
    pub keep_tags: bool,
    /// 无损增益（dB）。
    pub gain_db: Option<f64>,
    /// 增益后的真峰值上限（dBTP）。
    pub ceiling_dbtp: Option<f64>,
    /// 声道处理。
    pub channel_mode: ChannelMode,
    /// 是否把“假多声道”折混为立体声。
    pub downmix_fake_multichannel: bool,
}

impl Default for ConvertConfig {
    fn default() -> Self {
        ConvertConfig {
            spec: EncodeSpec {
                format: tf_core::model::AudioFormat::Flac,
                bit_depth: Some(24),
                sample_rate: None,
                channels: None,
                quality: tf_media::EncodeQuality::FlacLevel { level: 8 },
            },
            keep_tags: true,
            gain_db: None,
            ceiling_dbtp: Some(-1.0),
            channel_mode: ChannelMode::Keep,
            downmix_fake_multichannel: false,
        }
    }
}

/// 归一化页配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizeConfig {
    /// tf-core 的归一化参数。
    pub params: tf_core::limiter::NormalizeParams,
    /// 输出编码参数。
    pub spec: EncodeSpec,
    /// 是否保留标签。
    pub keep_tags: bool,
}

/// 转换/归一化的源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertSource {
    /// 源路径。
    pub path: PathBuf,
    /// 探测信息（决定默认位深/采样率与 DSP 路径选择）。
    pub media: MediaInfo,
}

/// 重命名条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenameEntry {
    /// 源路径。
    pub source: PathBuf,
    /// 目标文件名（含扩展名，已由 tf-core 合法化）。
    pub new_name: String,
}

/// 标签条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagsEntry {
    /// 源路径。
    pub source: PathBuf,
    /// 目标文件名（默认与源同名）。
    pub output_name: String,
    /// 要写入的最终标签。
    pub tags: Tags,
    /// 封面操作。
    #[serde(skip)]
    pub cover: Option<CoverAction>,
}

/// 封面操作。
#[derive(Debug, Clone, PartialEq)]
pub enum CoverAction {
    /// 写入/替换为给定封面。
    Set(CoverArt),
    /// 删除封面。
    Remove,
    /// 保持原样。
    Keep,
}

/// 转换任务载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertPayload {
    /// 编码参数。
    pub spec: EncodeSpec,
    /// 保留标签。
    pub keep_tags: bool,
    /// 无损增益。
    pub gain_db: Option<f64>,
    /// 真峰值上限。
    pub ceiling_dbtp: Option<f64>,
    /// 声道模式。
    pub channel_mode: ChannelMode,
    /// 假多声道折混。
    pub downmix_fake_multichannel: bool,
}

/// 归一化任务载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizePayload {
    /// 归一化参数。
    pub params: tf_core::limiter::NormalizeParams,
    /// 输出编码参数。
    pub spec: EncodeSpec,
    /// 保留标签。
    pub keep_tags: bool,
}

/// 标签任务载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagsPayload {
    /// 最终标签。
    pub tags: Tags,
    /// 封面操作（字节不参与序列化）。
    #[serde(skip)]
    pub cover: Option<CoverAction>,
}

/// 任务载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobPayload {
    /// 转换。
    Convert(ConvertPayload),
    /// 重命名（只复制 + 改名）。
    Rename,
    /// 归一化。
    Normalize(NormalizePayload),
    /// 标签。
    Tags(TagsPayload),
}

/// 一个待执行任务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// 稳定 id（uuid）。
    pub id: String,
    /// 类型。
    pub kind: JobKind,
    /// 源文件。
    pub input: PathBuf,
    /// 目标文件名（不含目录）。
    pub output_name: String,
    /// 展示标签。
    pub label: String,
    /// 载荷。
    pub payload: JobPayload,
}

impl Job {
    /// 源文件名。
    pub fn input_name(&self) -> String {
        self.input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// 源文件扩展名（不含点）。
    pub fn input_extension(&self) -> String {
        self.input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }
}

/// 任务清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobPlan {
    /// 输出目录。
    pub output_dir: PathBuf,
    /// 冲突策略。
    pub policy: ConflictPolicy,
    /// 任务列表。
    pub jobs: Vec<Job>,
}

impl JobPlan {
    /// 任务数。
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// 源文件列表（用于输出目录校验）。
    pub fn inputs(&self) -> Vec<PathBuf> {
        self.jobs.iter().map(|j| j.input.clone()).collect()
    }

    /// 导出任务清单（JSON），便于“导出/重跑”（§10.4）。
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// 从 JSON 恢复任务清单。
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|e| TfError::Input(format!("任务清单解析失败：{e}")))
    }

    /// 校验输出目录（非破坏性）。
    pub fn validate(&self) -> Result<()> {
        validate_output_dir(&self.output_dir, &self.inputs())
    }
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未命名".into())
}

fn new_job(kind: JobKind, input: PathBuf, output_name: String, payload: JobPayload) -> Job {
    let label = format!(
        "{}：{}",
        kind.label(),
        input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    Job {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        input,
        output_name,
        label,
        payload,
    }
}

/// 规划转换任务。
pub fn plan_convert(
    sources: Vec<ConvertSource>,
    config: &ConvertConfig,
    output_dir: &Path,
    policy: ConflictPolicy,
) -> Result<JobPlan> {
    if sources.is_empty() {
        return Err(TfError::Input("没有待转换的文件".into()));
    }
    let payload = JobPayload::Convert(ConvertPayload {
        spec: config.spec.clone(),
        keep_tags: config.keep_tags,
        gain_db: config.gain_db,
        ceiling_dbtp: config.ceiling_dbtp,
        channel_mode: config.channel_mode,
        downmix_fake_multichannel: config.downmix_fake_multichannel,
    });

    let jobs = sources
        .into_iter()
        .map(|src| {
            let output_name = format!("{}.{}", stem_of(&src.path), config.spec.extension());
            new_job(
                JobKind::Convert,
                src.path.clone(),
                output_name,
                payload.clone(),
            )
        })
        .collect::<Vec<_>>();

    let plan = JobPlan {
        output_dir: output_dir.to_path_buf(),
        policy,
        jobs,
    };
    plan.validate()?;
    Ok(plan)
}

/// 规划重命名任务（只改名，不转码）。
pub fn plan_rename(
    entries: Vec<RenameEntry>,
    output_dir: &Path,
    policy: ConflictPolicy,
) -> Result<JobPlan> {
    if entries.is_empty() {
        return Err(TfError::Input("没有待改名的文件".into()));
    }
    let jobs = entries
        .into_iter()
        .map(|e| new_job(JobKind::Rename, e.source, e.new_name, JobPayload::Rename))
        .collect::<Vec<_>>();
    let plan = JobPlan {
        output_dir: output_dir.to_path_buf(),
        policy,
        jobs,
    };
    plan.validate()?;
    Ok(plan)
}

/// 规划归一化任务。
pub fn plan_normalize(
    sources: Vec<ConvertSource>,
    config: &NormalizeConfig,
    output_dir: &Path,
    policy: ConflictPolicy,
) -> Result<JobPlan> {
    if sources.is_empty() {
        return Err(TfError::Input("没有待归一化的文件".into()));
    }
    let payload = JobPayload::Normalize(NormalizePayload {
        params: config.params.clone(),
        spec: config.spec.clone(),
        keep_tags: config.keep_tags,
    });
    let jobs = sources
        .into_iter()
        .map(|src| {
            let output_name = format!("{}.{}", stem_of(&src.path), config.spec.extension());
            new_job(
                JobKind::Normalize,
                src.path.clone(),
                output_name,
                payload.clone(),
            )
        })
        .collect::<Vec<_>>();
    let plan = JobPlan {
        output_dir: output_dir.to_path_buf(),
        policy,
        jobs,
    };
    plan.validate()?;
    Ok(plan)
}

/// 规划标签任务。
pub fn plan_tags(
    entries: Vec<TagsEntry>,
    output_dir: &Path,
    policy: ConflictPolicy,
) -> Result<JobPlan> {
    if entries.is_empty() {
        return Err(TfError::Input("没有待写标签的文件".into()));
    }
    let jobs = entries
        .into_iter()
        .map(|e| {
            let payload = JobPayload::Tags(TagsPayload {
                tags: e.tags,
                cover: e.cover,
            });
            new_job(JobKind::Tags, e.source, e.output_name, payload)
        })
        .collect::<Vec<_>>();
    let plan = JobPlan {
        output_dir: output_dir.to_path_buf(),
        policy,
        jobs,
    };
    plan.validate()?;
    Ok(plan)
}

/// 测试辅助：快速构造一个任务。
#[cfg(test)]
pub fn test_job(kind: JobKind, input: &str) -> Job {
    let payload = match kind {
        JobKind::Convert => JobPayload::Convert(ConvertPayload {
            spec: ConvertConfig::default().spec,
            keep_tags: true,
            gain_db: None,
            ceiling_dbtp: Some(-1.0),
            channel_mode: ChannelMode::Keep,
            downmix_fake_multichannel: false,
        }),
        JobKind::Normalize => JobPayload::Normalize(NormalizePayload {
            params: tf_core::limiter::NormalizeParams::default(),
            spec: ConvertConfig::default().spec,
            keep_tags: true,
        }),
        JobKind::Tags => JobPayload::Tags(TagsPayload {
            tags: Tags::default(),
            cover: None,
        }),
        JobKind::Rename => JobPayload::Rename,
    };
    new_job(kind, PathBuf::from(input), "out.flac".into(), payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tf_core::model::AudioFormat;

    fn source(path: &str) -> ConvertSource {
        ConvertSource {
            path: PathBuf::from(path),
            media: MediaInfo {
                path: PathBuf::from(path),
                format: Some(AudioFormat::Flac),
                container: "flac".into(),
                codec: "flac".into(),
                sample_rate: Some(44_100),
                bits_per_sample: Some(16),
                is_float: false,
                channels: Some(2),
                channel_layout: Some("stereo".into()),
                duration_secs: Some(3.0),
                frames: Some(132_300),
                bit_rate: None,
                size_bytes: 100,
            },
        }
    }

    fn out_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        (dir, out)
    }

    #[test]
    fn convert_plan_uses_target_extension_and_keeps_stem() {
        let (_root, out) = out_dir();
        let cfg = ConvertConfig::default();
        let plan = plan_convert(
            vec![source("C:/in/01 song.flac"), source("C:/in/02 song.wav")],
            &cfg,
            &out,
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan.jobs[0].output_name, "01 song.flac");
        assert_eq!(plan.jobs[1].output_name, "02 song.flac");
        assert_eq!(plan.jobs[0].kind, JobKind::Convert);
        assert!(plan.jobs[0].label.contains("格式转换"));
    }

    #[test]
    fn rename_plan_uses_provided_names() {
        let (_root, out) = out_dir();
        let plan = plan_rename(
            vec![RenameEntry {
                source: "C:/in/a.flac".into(),
                new_name: "Artist - Title.flac".into(),
            }],
            &out,
            ConflictPolicy::Rename,
        )
        .unwrap();
        assert_eq!(plan.jobs[0].output_name, "Artist - Title.flac");
        assert_eq!(plan.jobs[0].kind, JobKind::Rename);
        assert_eq!(plan.policy, ConflictPolicy::Rename);
    }

    #[test]
    fn empty_input_is_rejected() {
        let (_root, out) = out_dir();
        let err = plan_convert(vec![], &ConvertConfig::default(), &out, ConflictPolicy::Skip)
            .unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }

    #[test]
    fn output_dir_same_as_source_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.flac");
        std::fs::write(&src, b"x").unwrap();
        let err = plan_convert(
            vec![ConvertSource {
                path: src.clone(),
                media: source("x").media,
            }],
            &ConvertConfig::default(),
            dir.path(),
            ConflictPolicy::Skip,
        )
        .unwrap_err();
        assert!(err.to_string().contains("非破坏性"));
    }

    #[test]
    fn tags_plan_carries_final_tags() {
        let (_root, out) = out_dir();
        let tags = Tags {
            title: Some("T".into()),
            ..Tags::default()
        };
        let plan = plan_tags(
            vec![TagsEntry {
                source: "C:/in/a.flac".into(),
                output_name: "a.flac".into(),
                tags: tags.clone(),
                cover: Some(CoverAction::Set(CoverArt::new("image/png", vec![1, 2, 3]))),
            }],
            &out,
            ConflictPolicy::Skip,
        )
        .unwrap();
        match &plan.jobs[0].payload {
            JobPayload::Tags(payload) => {
                assert_eq!(payload.tags, tags);
                assert!(matches!(payload.cover, Some(CoverAction::Set(_))));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn plan_serializes_for_export_and_rerun() {
        let (_root, out) = out_dir();
        let plan = plan_normalize(
            vec![source("C:/in/a.flac")],
            &NormalizeConfig {
                params: tf_core::limiter::NormalizeParams::preset(-16.0),
                spec: ConvertConfig::default().spec,
                keep_tags: true,
            },
            &out,
            ConflictPolicy::Skip,
        )
        .unwrap();
        let json = plan.to_json();
        let back = JobPlan::from_json(&json).unwrap();
        assert_eq!(back.len(), plan.len());
        assert_eq!(back.jobs[0].output_name, plan.jobs[0].output_name);
        assert!(JobPlan::from_json("{oops").is_err());
    }

    #[test]
    fn normalize_plan_defaults_are_loudness_presets() {
        let (_root, out) = out_dir();
        let plan = plan_normalize(
            vec![source("C:/in/a.flac")],
            &NormalizeConfig {
                params: tf_core::limiter::NormalizeParams::default(),
                spec: ConvertConfig::default().spec,
                keep_tags: true,
            },
            &out,
            ConflictPolicy::Skip,
        )
        .unwrap();
        match &plan.jobs[0].payload {
            JobPayload::Normalize(p) => {
                assert_eq!(p.params.target_lufs, -14.0);
                assert_eq!(p.params.ceiling_dbtp, Some(-1.0));
                assert!(p.keep_tags);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn input_helpers_work() {
        let job = test_job(JobKind::Convert, "C:/in/Song.FLAC");
        assert_eq!(job.input_name(), "Song.FLAC");
        assert_eq!(job.input_extension(), "flac");
        assert_eq!(JobKind::Tags.label(), "标签修改");
    }
}
