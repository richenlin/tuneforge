//! Tauri 命令层（契约见 `docs/contracts/tauri-commands.md`）。
//!
//! 这一层只做“状态编排 + DTO 转换”，所有重活都在 `tf-core` / `tf-media` / `tf-tags` / `tf-jobs` 里。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tf_core::error::TfError;
use tf_core::limiter::{self, NormalizeParams};
use tf_core::model::{AudioFormat, MediaItem, SourceRating, Tags};
use tf_core::naming::{self, MissingFieldPolicy, SanitizeOptions};
use tf_core::{downmix, loudness, truepeak};
use tf_jobs::{
    plan_convert, plan_normalize, plan_rename, plan_tags, ConflictPolicy, ConvertConfig,
    ConvertSource, CoverAction, EventSink, FfmpegPipeline, JobPlan, LogLevel, QueueEvent,
    QueueRunner, RenameEntry, TagsEntry,
};
use tf_media::{decode, probe, FfmpegPaths};
use tf_tags as tags_io;

use crate::dto::{
    err_json, item_id, FieldEdit, FfmpegStatus, FormatOption, JobRequest, MeasureRow,
};
use crate::state::{cover_data_url, AppState};

fn map_err(error: TfError) -> String {
    err_json(&error)
}

/// 把 `QueueEvent` 转发到前端（§10.5 进度事件）。
struct EmitterSink {
    app: AppHandle,
}

impl EventSink for EmitterSink {
    fn on_job_started(&self, job: &tf_jobs::Job) {
        let _ = self.app.emit(
            "job:event",
            QueueEvent::Started {
                job_id: job.id.clone(),
                label: job.label.clone(),
                input: job.input.display().to_string(),
            },
        );
    }

    fn on_job_progress(&self, job_id: &str, percent: f32, stage: &str) {
        let _ = self.app.emit(
            "job:event",
            QueueEvent::Progress {
                job_id: job_id.to_string(),
                percent,
                stage: stage.to_string(),
            },
        );
    }

    fn on_job_log(&self, job_id: &str, level: LogLevel, message: &str) {
        let _ = self.app.emit(
            "job:event",
            QueueEvent::Log {
                job_id: job_id.to_string(),
                level,
                message: message.to_string(),
            },
        );
    }

    fn on_job_finished(&self, result: &tf_jobs::JobResult) {
        let _ = self.app.emit(
            "job:event",
            QueueEvent::Finished {
                result: result.clone(),
            },
        );
    }
}

// ---------------------------------------------------------------- ffmpeg 定位

/// ffmpeg 定位状态。
///
/// **不阻塞**：只读后台探测线程写入的快照；若尚未开始探测则顺带启动它。
#[tauri::command]
pub fn ffmpeg_status(app: AppHandle, state: State<'_, AppState>) -> FfmpegStatus {
    let snapshot = state.probe_state();
    if snapshot.probing() {
        state.start_probe(app);
    }
    FfmpegStatus::from_probe(&snapshot)
}

/// 指定 ffmpeg 路径（目录或可执行文件）；立即返回并在后台重新探测。
#[tauri::command]
pub fn set_ffmpeg_path(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> FfmpegStatus {
    let trimmed = path.trim();
    state.set_ffmpeg_override(if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    });
    state.start_probe(app);
    let snapshot = state.probe_state();
    FfmpegStatus::from_probe(&snapshot)
}

// ---------------------------------------------------------------- 扫描与列表

fn build_item(paths: &FfmpegPaths, file: &Path) -> MediaItem {
    let id = item_id(&file.to_path_buf());
    match probe::read_media_info(paths, file) {
        Ok(media) => {
            let (tags, cover) = match tags_io::read_tags_and_cover(file) {
                Ok((t, c)) => (t, c),
                Err(e) => {
                    tracing::warn!(path = %file.display(), error = %e, "读取标签失败");
                    (Tags::default(), None)
                }
            };
            MediaItem {
                id,
                path: file.to_path_buf(),
                file_name: file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                rating: SourceRating::from_media(&media),
                media: Some(media),
                tags,
                has_cover: cover.is_some(),
                error: None,
                loudness_lufs: None,
                true_peak_dbtp: None,
                fake_multichannel: false,
            }
        }
        Err(e) => MediaItem {
            id,
            path: file.to_path_buf(),
            file_name: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            media: None,
            tags: Tags::default(),
            has_cover: false,
            error: Some(e.to_string()),
            loudness_lufs: None,
            true_peak_dbtp: None,
            fake_multichannel: false,
            rating: None,
        },
    }
}

/// 扫描输入（文件夹/文件）。
///
/// 每个文件要 spawn 一次 ffprobe，因此放到阻塞线程池执行，避免卡住 UI 线程。
#[tauri::command]
pub async fn scan_inputs(
    state: State<'_, AppState>,
    paths: Vec<String>,
    recursive: bool,
) -> Result<Vec<MediaItem>, String> {
    let (ffmpeg, _caps) = state.require_toolchain().map_err(map_err)?;
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let items = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<MediaItem>, String> {
        let files = tf_media::scan::scan_inputs(&inputs, recursive).map_err(map_err)?;
        Ok(files.iter().map(|file| build_item(&ffmpeg, file)).collect())
    })
    .await
    .map_err(|e| format!("扫描线程异常：{e}"))??;
    state.extend_items(items.clone());
    Ok(items)
}

/// 当前列表。
#[tauri::command]
pub fn list_items(state: State<'_, AppState>) -> Vec<MediaItem> {
    state.items()
}

/// 清空列表。
#[tauri::command]
pub fn clear_items(state: State<'_, AppState>) {
    state.clear_items();
}

/// 从列表移除。
#[tauri::command]
pub fn remove_items(state: State<'_, AppState>, ids: Vec<String>) -> Vec<MediaItem> {
    state.remove_items(&ids)
}

// ---------------------------------------------------------------- 参数

/// 转换页格式下拉。
#[tauri::command]
pub fn encode_options(state: State<'_, AppState>) -> Result<Vec<FormatOption>, String> {
    let (_paths, caps) = state.require_toolchain().map_err(map_err)?;
    let options = AudioFormat::output_formats()
        .into_iter()
        .map(|format| {
            let spec = tf_media::default_spec(format, &dummy_media()).ok();
            FormatOption {
                format: format.id().to_string(),
                label: format.label().to_string(),
                extension: format.extension().to_string(),
                lossless: format.is_lossless(),
                available: caps.supports(format),
                reason: if caps.supports(format) {
                    None
                } else {
                    tf_media::Capabilities::required_encoder(format)
                        .map(|enc| format!("当前 FFmpeg 缺少编码器 {enc}"))
                },
                default_bit_depth: spec.as_ref().and_then(|s| s.bit_depth),
                default_quality: spec
                    .as_ref()
                    .map(|s| s.quality.summary())
                    .unwrap_or_else(|| "-".into()),
            }
        })
        .collect();
    Ok(options)
}

fn dummy_media() -> tf_core::model::MediaInfo {
    tf_core::model::MediaInfo {
        path: PathBuf::new(),
        format: None,
        container: String::new(),
        codec: String::new(),
        sample_rate: None,
        bits_per_sample: None,
        is_float: false,
        channels: None,
        channel_layout: None,
        duration_secs: None,
        frames: None,
        bit_rate: None,
        size_bytes: 0,
    }
}

/// 转换页采样率下拉：按目标格式给候选值 + 推荐值（`value = null` 表示保持源采样率）。
#[tauri::command]
pub fn sample_rate_options(
    state: State<'_, AppState>,
    format: String,
    item_id: Option<String>,
) -> Result<Vec<tf_media::SampleRateOption>, String> {
    let target = AudioFormat::from_id(&format).ok_or_else(|| format!("未知格式 {format}"))?;
    // 以当前参考条目的源采样率为准做推荐（未选中时为未知）
    let source_rate = item_id
        .and_then(|id| state.item(&id))
        .and_then(|item| item.media)
        .and_then(|media| media.sample_rate);
    Ok(tf_media::sample_rate_options(target, source_rate))
}

/// 按设计方案 §6.2 给出推荐编码参数。
#[tauri::command]
pub fn default_encode_spec(
    state: State<'_, AppState>,
    format: String,
    item_id: Option<String>,
) -> Result<tf_media::EncodeSpec, String> {
    let target = AudioFormat::from_id(&format)
        .ok_or_else(|| format!("未知格式 {format}"))?;
    let media = item_id
        .and_then(|id| state.item(&id))
        .and_then(|item| item.media)
        .unwrap_or_else(dummy_media);
    tf_media::default_spec(target, &media).map_err(map_err)
}

/// 逐文件测量集成响度与真峰值（归一化页）。
///
/// 需要整曲解码 + DSP，属于最重的命令，必须放到阻塞线程池。
#[tauri::command]
pub async fn measure_loudness(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<MeasureRow>, String> {
    let (paths, _caps) = state.require_toolchain().map_err(map_err)?;
    let items = state.select(&ids);
    let rows = tauri::async_runtime::spawn_blocking(move || {
        items.iter().map(|item| measure_one(&paths, item)).collect::<Vec<_>>()
    })
    .await
    .map_err(|e| format!("测量线程异常：{e}"))?;

    for row in &rows {
        let (lufs, tp, fake) = (row.loudness_lufs, row.true_peak_dbtp, row.fake_multichannel);
        state.update_items(std::slice::from_ref(&row.id), |item| {
            item.loudness_lufs = lufs;
            item.true_peak_dbtp = tp;
            item.fake_multichannel = fake;
        });
    }
    Ok(rows)
}

fn measure_one(paths: &FfmpegPaths, item: &MediaItem) -> MeasureRow {
    let mut row = MeasureRow {
        id: item.id.clone(),
        file_name: item.file_name.clone(),
        loudness_lufs: None,
        true_peak_dbtp: None,
        sample_peak_dbfs: None,
        fake_multichannel: false,
        note: None,
        error: None,
    };
    let Some(media) = &item.media else {
        row.error = Some("文件未成功探测".into());
        return row;
    };
    let opts = decode::DecodeOptions {
        source_duration_secs: media.duration_secs,
        ..Default::default()
    };
    match decode::decode_to_buffer(paths, &item.path, &opts, None) {
        Ok(buffer) => {
            let m = loudness::measure(&buffer);
            row.loudness_lufs = m.integrated_lufs;
            row.sample_peak_dbfs = Some(m.sample_peak_dbfs);
            row.true_peak_dbtp = Some(truepeak::true_peak_dbtp(&buffer));
            let fake_report =
                downmix::detect_fake_multichannel(&buffer, downmix::DEFAULT_SILENCE_THRESHOLD_DBFS);
            row.fake_multichannel = fake_report.is_fake;
            if fake_report.is_fake {
                row.note = Some(fake_report.note());
            }
        }
        Err(e) => row.error = Some(e.to_string()),
    }
    row
}

/// 重命名模板预设。
#[tauri::command]
pub fn template_presets() -> Vec<serde_json::Value> {
    naming::presets()
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "label": p.label,
                "template": p.template,
            })
        })
        .collect()
}

/// 重命名预览（不落盘）。
#[tauri::command]
pub fn preview_rename(
    state: State<'_, AppState>,
    template: String,
    missing: MissingFieldPolicy,
    sanitize: SanitizeOptions,
    output_dir: String,
    ids: Vec<String>,
) -> Vec<naming::RenamePreviewItem> {
    let items = state.select(&ids);
    let options = naming::NamingOptions { missing, sanitize };
    let dir = PathBuf::from(&output_dir);
    let inputs: Vec<naming::RenameInput> = items
        .iter()
        .map(|item| naming::RenameInput {
            path: item.path.clone(),
            file_name: item.file_name.clone(),
            tags: item.tags.clone(),
        })
        .collect();
    naming::preview_rename(&inputs, &template, &options, &dir, &|p: &Path| p.exists())
}

// ---------------------------------------------------------------- 标签编辑

/// 批量编辑字段（只改内存，任务执行时才写盘）。
#[tauri::command]
pub fn update_tags(
    state: State<'_, AppState>,
    ids: Vec<String>,
    edits: Vec<FieldEdit>,
) -> Result<Vec<MediaItem>, String> {
    for edit in &edits {
        // 提前校验数字字段，避免写入时才失败
        let mut probe_tags = Tags::default();
        edit.field
            .set(&mut probe_tags, edit.value.clone())
            .map_err(map_err)?;
    }
    let items = state.update_items(&ids, |item| {
        for edit in &edits {
            if let Err(e) = edit.field.set(&mut item.tags, edit.value.clone()) {
                tracing::warn!(error = %e, "字段校验失败");
            }
        }
        item.tags = item.tags.clone().normalized();
    });
    Ok(items)
}

/// 批量查找替换。
#[tauri::command]
pub fn replace_in_tags(
    state: State<'_, AppState>,
    ids: Vec<String>,
    field: tf_core::model::TagField,
    find: String,
    replace: String,
    case_insensitive: bool,
) -> Vec<MediaItem> {
    state.update_items(&ids, |item| {
        naming::replace_in_field(&mut item.tags, field, &find, &replace, case_insensitive);
    })
}

/// 从文件名反推标签。
#[tauri::command]
pub fn guess_tags(
    state: State<'_, AppState>,
    ids: Vec<String>,
    template: String,
) -> Vec<MediaItem> {
    state.update_items(&ids, |item| {
        let guessed = naming::guess_tags_from_filename(&item.file_name, &template);
        if !guessed.is_empty() {
            item.tags = guessed;
        }
    })
}

/// 封面预览（data URL）。
#[tauri::command]
pub fn cover_preview(state: State<'_, AppState>, item_id: String) -> Result<Option<String>, String> {
    let item = state
        .item(&item_id)
        .ok_or_else(|| "条目不存在".to_string())?;
    match tags_io::read_first_cover(&item.path) {
        Ok(Some(cover)) => Ok(Some(cover_data_url(&cover))),
        Ok(None) => Ok(None),
        Err(e) => Err(map_err(e)),
    }
}

/// 嵌入/替换封面。
#[tauri::command]
pub fn set_cover(
    state: State<'_, AppState>,
    item_id: String,
    image_path: String,
) -> Result<Option<String>, String> {
    let cover = tags_io::cover_from_file(Path::new(&image_path)).map_err(map_err)?;
    let url = cover_data_url(&cover);
    state.stage_cover(&item_id, CoverAction::Set(cover));
    state.update_items(&[item_id], |item| item.has_cover = true);
    Ok(Some(url))
}

/// 删除封面。
#[tauri::command]
pub fn remove_cover(state: State<'_, AppState>, item_id: String) {
    state.stage_cover(&item_id, CoverAction::Remove);
    state.update_items(&[item_id], |item| item.has_cover = false);
}

/// 导出封面。
#[tauri::command]
pub fn export_cover(
    state: State<'_, AppState>,
    item_id: String,
    index: usize,
    out_dir: String,
) -> Result<String, String> {
    let item = state
        .item(&item_id)
        .ok_or_else(|| "条目不存在".to_string())?;
    let path = tags_io::export_cover(&item.path, index, Path::new(&out_dir)).map_err(map_err)?;
    Ok(path.display().to_string())
}

// ---------------------------------------------------------------- 任务

fn build_plan(
    state: &AppState,
    paths: &FfmpegPaths,
    request: &JobRequest,
) -> Result<JobPlan, String> {
    let output_dir = PathBuf::from(request.output_dir());
    let policy = request.policy();
    let items = state.select(request.ids());
    if items.is_empty() {
        return Err("请选择要处理的文件".to_string());
    }

    match request {
        JobRequest::Convert { config, .. } => {
            // 采样率必须落在目标格式支持范围内（例如 Opus 只能 48 kHz、MP3 最大 48 kHz）
            if let Some(rate) = config.spec.sample_rate {
                if !tf_media::is_sample_rate_supported(config.spec.format, rate) {
                    return Err(format!(
                        "{} 不支持 {} Hz 采样率（最大 {} Hz）",
                        config.spec.format.label(),
                        tf_media::format_rate(rate),
                        tf_media::format_rate(tf_media::sample_rate_limit(config.spec.format))
                    ));
                }
            }
            let sources = to_sources(&items)?;
            plan_convert(sources, &ConvertConfig::from(config.clone()), &output_dir, policy)
                .map_err(map_err)
        }
        JobRequest::Rename {
            template,
            missing,
            sanitize,
            ..
        } => {
            let options = naming::NamingOptions {
                missing: *missing,
                sanitize: sanitize.clone(),
            };
            let inputs: Vec<naming::RenameInput> = items
                .iter()
                .map(|item| naming::RenameInput {
                    path: item.path.clone(),
                    file_name: item.file_name.clone(),
                    tags: item.tags.clone(),
                })
                .collect();
            let previews =
                naming::preview_rename(&inputs, template, &options, &output_dir, &|p: &Path| {
                    p.exists()
                });
            let entries: Vec<RenameEntry> = previews
                .iter()
                .filter(|p| !p.skip && p.changed)
                .map(|p| RenameEntry {
                    source: p.source_path.clone(),
                    new_name: p.new_name.clone(),
                })
                .collect();
            plan_rename(entries, &output_dir, policy).map_err(map_err)
        }
        JobRequest::Normalize {
            params,
            format,
            bit_depth,
            keep_tags,
            ..
        } => {
            let sources = to_sources(&items)?;
            let media = sources[0].media.clone();
            let mut spec = tf_media::default_spec(*format, &media).map_err(map_err)?;
            spec.bit_depth = *bit_depth;
            plan_normalize(
                sources,
                &tf_jobs::NormalizeConfig {
                    params: params.clone(),
                    spec,
                    keep_tags: *keep_tags,
                },
                &output_dir,
                policy,
            )
            .map_err(map_err)
        }
        JobRequest::Tags { .. } => {
            let entries: Vec<TagsEntry> = items
                .iter()
                .map(|item| TagsEntry {
                    source: item.path.clone(),
                    output_name: item.file_name.clone(),
                    tags: item.tags.clone(),
                    cover: state.take_cover(&item.id),
                })
                .collect();
            plan_tags(entries, &output_dir, policy).map_err(map_err)
        }
    }
    .and_then(|plan| {
        let _ = paths;
        if plan.is_empty() {
            Err("没有需要处理的任务（可能都已跳过）".to_string())
        } else {
            Ok(plan)
        }
    })
}

fn to_sources(items: &[MediaItem]) -> Result<Vec<ConvertSource>, String> {
    let mut sources = Vec::with_capacity(items.len());
    for item in items {
        let Some(media) = item.media.clone() else {
            return Err(format!("{} 未成功探测，无法处理", item.file_name));
        };
        sources.push(ConvertSource {
            path: item.path.clone(),
            media,
        });
    }
    Ok(sources)
}

/// 开始任务（后台执行，事件推送进度）。
#[tauri::command]
pub fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    request: JobRequest,
) -> Result<String, String> {
    let (paths, caps) = state.require_toolchain().map_err(map_err)?;
    let plan = build_plan(&state, &paths, &request)?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = state.register_job(&job_id, plan.len());
    let registry = state.job_registry();
    let pipeline = Arc::new(FfmpegPipeline::with_capabilities(paths, &caps));
    let handle = app.clone();
    let id_for_thread = job_id.clone();

    std::thread::spawn(move || {
        let sink: Arc<dyn EventSink> = Arc::new(EmitterSink { app: handle.clone() });
        let result = QueueRunner::new(0).run(&plan, pipeline.as_ref(), sink, cancel);
        {
            let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = guard.get_mut(&id_for_thread) {
                slot.finished = true;
                slot.report = result.as_ref().ok().cloned();
            }
        }
        match result {
            Ok(report) => {
                let _ = handle.emit(
                    "job:done",
                    serde_json::json!({ "jobId": id_for_thread, "report": report }),
                );
            }
            Err(e) => {
                let _ = handle.emit(
                    "job:done",
                    serde_json::json!({ "jobId": id_for_thread, "error": e.to_string() }),
                );
            }
        }
    });

    Ok(job_id)
}

/// 取消任务。
#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.cancel_job(&job_id).map_err(map_err)
}

/// 任务报告。
#[tauri::command]
pub fn job_report(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<tf_jobs::JobReport, String> {
    state
        .job_report(&job_id)
        .ok_or_else(|| "任务尚未结束或不存在".to_string())
}

/// 导出任务清单（JSON，可重跑）。
#[tauri::command]
pub fn export_plan(
    state: State<'_, AppState>,
    request: JobRequest,
    path: String,
) -> Result<String, String> {
    let (paths, _caps) = state.require_toolchain().map_err(map_err)?;
    let plan = build_plan(&state, &paths, &request)?;
    std::fs::write(&path, plan.to_json()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 导出结果（CSV/JSON）。
#[tauri::command]
pub fn export_report(
    state: State<'_, AppState>,
    job_id: String,
    format: String,
    path: String,
) -> Result<String, String> {
    let report = state
        .job_report(&job_id)
        .ok_or_else(|| "任务尚未结束或不存在".to_string())?;
    let content = match format.to_ascii_lowercase().as_str() {
        "csv" => report.to_csv(),
        "json" => report.to_json(),
        other => return Err(format!("不支持的导出格式 {other}（仅 csv/json）")),
    };
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 在资源管理器中打开路径。
#[tauri::command]
pub fn open_in_explorer(path: String) -> Result<(), String> {
    let target = PathBuf::from(&path);
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        c.arg(&target);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(&target);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&target);
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// 应用信息（关于页）。
#[tauri::command]
pub fn app_info() -> serde_json::Value {
    serde_json::json!({
        "name": "Tuneforge",
        "version": env!("CARGO_PKG_VERSION"),
        "features": ["格式转换", "文件名标准化", "音量归一化", "标签修改"],
        "notes": [
            "APE 只能作为输入（FFmpeg 无 APE 编码器）",
            "不做 PCM → DSD（仅解码 DSD）",
            "所有操作只读源文件，结果输出到新文件夹"
        ]
    })
}

/// 便捷：解码一段音频（调试/预览用，最多 30 秒）。
#[tauri::command]
pub async fn preview_decode(
    state: State<'_, AppState>,
    item_id: String,
    seconds: Option<f64>,
) -> Result<serde_json::Value, String> {
    let (paths, _caps) = state.require_toolchain().map_err(map_err)?;
    let item = state
        .item(&item_id)
        .ok_or_else(|| "条目不存在".to_string())?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let opts = decode::DecodeOptions {
            duration_secs: Some(seconds.unwrap_or(30.0)),
            ..Default::default()
        };
        let buffer = decode::decode_to_buffer(&paths, &item.path, &opts, None).map_err(map_err)?;
        let m = loudness::measure(&buffer);
        Ok(serde_json::json!({
            "sampleRate": buffer.sample_rate,
            "channels": buffer.channels,
            "frames": buffer.frames,
            "loudnessLufs": m.integrated_lufs,
            "truePeakDbtp": truepeak::true_peak_dbtp(&buffer),
        }))
    })
    .await
    .map_err(|e| format!("解码线程异常：{e}"))?
}

/// 预估增益（归一化页即时提示，不需要完整解码）。
#[tauri::command]
pub fn gain_preview(
    state: State<'_, AppState>,
    ids: Vec<String>,
    params: NormalizeParams,
) -> Vec<serde_json::Value> {
    let _ = &state;
    state
        .select(&ids)
        .iter()
        .map(|item| {
            let gain = item
                .loudness_lufs
                .map(|l| params.target_lufs - l)
                .unwrap_or(0.0);
            serde_json::json!({
                "id": item.id,
                "fileName": item.file_name,
                "measuredLufs": item.loudness_lufs,
                "truePeakDbtp": item.true_peak_dbtp,
                "gainDb": gain,
                "needsMeasuring": item.loudness_lufs.is_none(),
            })
        })
        .collect()
}

/// 冲突策略标签（前端展示）。
#[tauri::command]
pub fn conflict_policies() -> Vec<serde_json::Value> {
    [
        ConflictPolicy::Skip,
        ConflictPolicy::Overwrite,
        ConflictPolicy::Rename,
    ]
    .into_iter()
    .map(|p| {
        serde_json::json!({
            "id": match p {
                ConflictPolicy::Skip => "skip",
                ConflictPolicy::Overwrite => "overwrite",
                ConflictPolicy::Rename => "rename",
            },
            "label": p.label(),
        })
    })
    .collect()
}

/// 默认归一化参数（D8）。
#[tauri::command]
pub fn default_normalize_params() -> NormalizeParams {
    limiter::NormalizeParams::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tf_core::model::Tags;

    fn item(path: &str) -> MediaItem {
        let p = PathBuf::from(path);
        MediaItem {
            id: item_id(&p),
            path: p.clone(),
            file_name: p.file_name().unwrap().to_string_lossy().into_owned(),
            media: None,
            tags: Tags::default(),
            has_cover: false,
            error: None,
            loudness_lufs: None,
            true_peak_dbtp: None,
            fake_multichannel: false,
            rating: None,
        }
    }

    #[test]
    fn item_ids_are_stable_and_case_insensitive() {
        let a = item_id(&PathBuf::from("C:/Music/Song.FLAC"));
        let b = item_id(&PathBuf::from("c:/music/song.flac"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert_ne!(a, item_id(&PathBuf::from("C:/Music/Other.FLAC")));
    }

    #[test]
    fn err_json_carries_category() {
        let text = err_json(&TfError::Decode("ffmpeg 失败".into()));
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["category"], "decode");
        assert!(parsed["message"].as_str().unwrap().contains("ffmpeg"));
        assert_eq!(parsed["categoryLabel"], "解码失败");
    }

    #[test]
    fn state_tracks_items_and_selection() {
        let state = AppState::default();
        state.extend_items(vec![item("C:/a.flac"), item("C:/b.flac")]);
        assert_eq!(state.items().len(), 2);
        assert_eq!(state.select(&[]).len(), 2);
        let first = state.items()[0].id.clone();
        assert_eq!(state.select(&[first.clone()]).len(), 1);
        let updated = state.update_items(&[first.clone()], |i| {
            i.tags.title = Some("新标题".into());
        });
        assert_eq!(updated[0].tags.title.as_deref(), Some("新标题"));
        state.remove_items(&[first.clone()]);
        assert_eq!(state.items().len(), 1);
        state.clear_items();
        assert!(state.items().is_empty());
    }

    #[test]
    fn job_registry_supports_cancel_and_missing_lookup() {
        let state = AppState::default();
        let token = state.register_job("job-1", 3);
        assert!(!token.is_cancelled());
        state.cancel_job("job-1").unwrap();
        assert!(token.is_cancelled());
        assert!(state.cancel_job("nope").is_err());
        assert!(state.job_report("job-1").is_none());
    }

    #[test]
    fn pending_cover_actions_can_be_staged_and_taken() {
        let state = AppState::default();
        state.stage_cover("id1", CoverAction::Remove);
        assert!(matches!(state.take_cover("id1"), Some(CoverAction::Remove)));
        assert!(state.take_cover("id1").is_none());
    }

    #[test]
    fn dummy_media_is_usable_for_default_specs() {
        let spec = tf_media::default_spec(AudioFormat::Wav, &dummy_media()).unwrap();
        assert_eq!(spec.bit_depth, Some(24));
    }

    #[test]
    fn conflict_policy_list_is_chinese() {
        let list = conflict_policies();
        assert_eq!(list.len(), 3);
        assert!(list[0]["label"].as_str().unwrap().contains("跳过"));
    }
}
