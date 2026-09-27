//! 应用状态：媒体列表、ffmpeg 探测（后台）、任务注册表。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::{AppHandle, Emitter, Manager};
use tf_core::error::{Result, TfError};
use tf_core::model::{CoverArt, MediaItem};
use tf_jobs::{CancelToken, CoverAction, JobReport};
use tf_media::{Capabilities, FfmpegPaths};

use crate::dto::FfmpegStatus;
use crate::probe_cache;

/// 探测进度事件名（前端监听后刷新状态胶囊）。
pub const FFMPEG_STATUS_EVENT: &str = "ffmpeg:status";

/// ffmpeg 探测状态机。
#[derive(Debug, Clone)]
pub enum ProbeState {
    /// 尚未开始（或已被失效，需要重新探测）。
    Idle,
    /// 正在后台探测。
    Probing,
    /// 可用。
    Ready {
        /// 可执行文件路径。
        paths: FfmpegPaths,
        /// 能力快照。
        caps: Box<Capabilities>,
        /// 是否命中磁盘缓存（命中则不 spawn 子进程）。
        cached: bool,
        /// 探测耗时（毫秒）。
        elapsed_ms: u64,
    },
    /// 探测失败（中文原因）。
    Failed(String),
}

impl ProbeState {
    /// 是否仍在探测（未就绪）。
    pub fn probing(&self) -> bool {
        matches!(self, ProbeState::Idle | ProbeState::Probing)
    }

    /// 就绪时的路径与能力。
    pub fn ready(&self) -> Option<(&FfmpegPaths, &Capabilities)> {
        match self {
            ProbeState::Ready { paths, caps, .. } => Some((paths, caps)),
            _ => None,
        }
    }
}

/// 一个任务的运行时状态。
#[derive(Debug)]
pub struct JobSlot {
    /// 取消令牌。
    pub cancel: CancelToken,
    /// 结果（完成后填充）。
    pub report: Option<JobReport>,
    /// 计划里的任务数。
    pub planned: usize,
    /// 是否已经结束。
    pub finished: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// 应用共享状态。
#[derive(Debug, Default)]
pub struct AppState {
    /// 已扫描的媒体列表（保持扫描顺序）。
    items: Mutex<Vec<MediaItem>>,
    /// 用户覆盖的 ffmpeg 目录。
    ffmpeg_override: Mutex<Option<PathBuf>>,
    /// ffmpeg 探测状态（后台线程写，命令层只读）。
    probe: Arc<Mutex<ProbeState>>,
    /// 待写入的封面操作。
    pending_covers: Mutex<HashMap<String, CoverAction>>,
    /// 任务注册表。
    jobs: Arc<Mutex<HashMap<String, JobSlot>>>,
}

impl Default for ProbeState {
    fn default() -> Self {
        ProbeState::Idle
    }
}

impl AppState {
    /// 任务注册表（可克隆进后台线程）。
    pub fn job_registry(&self) -> Arc<Mutex<HashMap<String, JobSlot>>> {
        Arc::clone(&self.jobs)
    }

    // ------------------------------------------------------------ ffmpeg 探测

    /// 当前探测状态快照。
    pub fn probe_state(&self) -> ProbeState {
        lock(&self.probe).clone()
    }

    /// 尝试把状态置为「探测中」；返回 `true` 表示调用方应该启动线程。
    pub fn begin_probe(&self) -> bool {
        let mut guard = lock(&self.probe);
        match &*guard {
            ProbeState::Probing => false,
            _ => {
                *guard = ProbeState::Probing;
                true
            }
        }
    }

    /// 使探测结果失效（切换 ffmpeg 路径后调用）。
    pub fn invalidate_probe(&self) {
        *lock(&self.probe) = ProbeState::Idle;
    }

    /// 在后台线程探测 ffmpeg（幂等：已在探测中则直接返回）。
    ///
    /// 关键点：探测会 spawn 134 MB 的 ffmpeg（冷启动可能被 Defender 扫描数秒~数十秒），
    /// 因此绝不能放在命令线程（主线程）上执行；完成后通过 `ffmpeg:status` 事件通知前端。
    pub fn start_probe(&self, app: AppHandle) {
        if !self.begin_probe() {
            return;
        }
        let override_dir = self.ffmpeg_override();
        let cache_path = app
            .path()
            .app_cache_dir()
            .ok()
            .map(|dir| probe_cache::cache_file(&dir));
        let probe = Arc::clone(&self.probe);
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let next = match probe_toolchain(override_dir.as_deref(), cache_path.as_deref()) {
                Ok((paths, caps, cached)) => {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    if cached {
                        tracing::info!(elapsed_ms, "FFmpeg 探测命中缓存（未启动子进程）");
                    } else {
                        tracing::info!(elapsed_ms, version = %caps.version_line, "FFmpeg 探测完成");
                    }
                    ProbeState::Ready {
                        paths,
                        caps: Box::new(caps),
                        cached,
                        elapsed_ms,
                    }
                }
                Err(message) => {
                    tracing::warn!(elapsed_ms = started.elapsed().as_millis() as u64, %message, "FFmpeg 探测失败");
                    ProbeState::Failed(message)
                }
            };
            *lock(&probe) = next.clone();
            let status = FfmpegStatus::from_probe(&next);
            let _ = app.emit(FFMPEG_STATUS_EVENT, status);
        });
    }

    /// 就绪时的 ffmpeg 路径与能力；未就绪时返回可读的中文错误。
    pub fn require_toolchain(&self) -> Result<(FfmpegPaths, Capabilities)> {
        match self.probe_state() {
            ProbeState::Ready { paths, caps, .. } => Ok((paths, (*caps).clone())),
            ProbeState::Probing | ProbeState::Idle => Err(TfError::Unsupported(
                "正在检测 FFmpeg，请稍候片刻再试".into(),
            )),
            ProbeState::Failed(message) => Err(TfError::Unsupported(message)),
        }
    }

    /// 设置 ffmpeg 覆盖路径（会使缓存失效并重新探测）。
    pub fn set_ffmpeg_override(&self, path: Option<PathBuf>) {
        *lock(&self.ffmpeg_override) = path;
        self.invalidate_probe();
    }

    /// ffmpeg 覆盖路径。
    pub fn ffmpeg_override(&self) -> Option<PathBuf> {
        lock(&self.ffmpeg_override).clone()
    }

    // ------------------------------------------------------------ 列表

    /// 列表快照。
    pub fn items(&self) -> Vec<MediaItem> {
        lock(&self.items).clone()
    }

    /// 替换整个列表。
    pub fn set_items(&self, items: Vec<MediaItem>) {
        *lock(&self.items) = items;
    }

    /// 追加条目（按 id 去重，后到的覆盖旧的）。
    pub fn extend_items(&self, new_items: Vec<MediaItem>) {
        let mut guard = lock(&self.items);
        for item in new_items {
            match guard.iter_mut().find(|i| i.id == item.id) {
                Some(existing) => *existing = item,
                None => guard.push(item),
            }
        }
    }

    /// 按 id 取条目。
    pub fn item(&self, id: &str) -> Option<MediaItem> {
        lock(&self.items).iter().find(|i| i.id == id).cloned()
    }

    /// 按 id 列表取条目（空表示全部）。
    pub fn select(&self, ids: &[String]) -> Vec<MediaItem> {
        let guard = lock(&self.items);
        if ids.is_empty() {
            return guard.clone();
        }
        guard.iter().filter(|i| ids.contains(&i.id)).cloned().collect()
    }

    /// 对选中条目就地更新。
    pub fn update_items<F>(&self, ids: &[String], mut f: F) -> Vec<MediaItem>
    where
        F: FnMut(&mut MediaItem),
    {
        let mut guard = lock(&self.items);
        for item in guard.iter_mut() {
            if ids.is_empty() || ids.contains(&item.id) {
                f(item);
            }
        }
        guard.clone()
    }

    /// 删除条目。
    pub fn remove_items(&self, ids: &[String]) -> Vec<MediaItem> {
        let mut guard = lock(&self.items);
        guard.retain(|i| !ids.contains(&i.id));
        guard.clone()
    }

    /// 清空列表与待写入封面。
    pub fn clear_items(&self) {
        lock(&self.items).clear();
        lock(&self.pending_covers).clear();
    }

    // ------------------------------------------------------------ 封面

    /// 记录待写入的封面操作。
    pub fn stage_cover(&self, item_id: &str, action: CoverAction) {
        lock(&self.pending_covers).insert(item_id.to_string(), action);
    }

    /// 取出待写入的封面操作。
    pub fn take_cover(&self, item_id: &str) -> Option<CoverAction> {
        lock(&self.pending_covers).remove(item_id)
    }

    // ------------------------------------------------------------ 任务

    /// 注册任务并返回取消令牌。
    pub fn register_job(&self, job_id: &str, planned: usize) -> CancelToken {
        let token = CancelToken::new();
        lock(&self.jobs).insert(
            job_id.to_string(),
            JobSlot {
                cancel: token.clone(),
                report: None,
                planned,
                finished: false,
            },
        );
        token
    }

    /// 取消任务。
    pub fn cancel_job(&self, job_id: &str) -> Result<()> {
        let guard = lock(&self.jobs);
        match guard.get(job_id) {
            Some(slot) => {
                slot.cancel.cancel();
                Ok(())
            }
            None => Err(TfError::Input(format!("找不到任务 {job_id}"))),
        }
    }

    /// 任务报告。
    pub fn job_report(&self, job_id: &str) -> Option<JobReport> {
        lock(&self.jobs).get(job_id).and_then(|s| s.report.clone())
    }
}

/// 探测实现：定位 → 查缓存 → （未命中）查询能力 → 回写缓存。
fn probe_toolchain(
    override_dir: Option<&Path>,
    cache_path: Option<&Path>,
) -> std::result::Result<(FfmpegPaths, Capabilities, bool), String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let resource_dir = exe_dir.as_deref().and_then(|dir| dir.parent().map(Path::to_path_buf));
    let candidates = tf_media::locate::candidate_dirs(exe_dir.as_deref(), resource_dir.as_deref());
    let paths = tf_media::locate::discover(override_dir, &candidates).map_err(|e| e.to_string())?;

    if let Some(cache_path) = cache_path {
        if let Some(entry) = probe_cache::load(cache_path) {
            if entry.matches(&paths) {
                return Ok((paths, entry.capabilities, true));
            }
        }
    }

    let caps = tf_media::capabilities::query(&paths).map_err(|e| e.to_string())?;
    if let Some(cache_path) = cache_path {
        if let Some(entry) = probe_cache::ProbeCache::new(&paths, &caps) {
            probe_cache::save(cache_path, &entry);
        }
    }
    Ok((paths, caps, false))
}

/// 封面数据转 data URL（前端预览）。
pub fn cover_data_url(cover: &CoverArt) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&cover.data);
    format!("data:{};base64,{}", cover.mime, encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tf_core::model::Tags;

    fn item(path: &str) -> MediaItem {
        let p = PathBuf::from(path);
        MediaItem {
            id: path.to_lowercase(),
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

    fn caps() -> Capabilities {
        Capabilities {
            version_line: "ffmpeg version 8.1".into(),
            configuration: String::new(),
            encoders: vec!["flac".into()],
            raw_encoders: String::new(),
        }
    }

    fn ready_state() -> ProbeState {
        ProbeState::Ready {
            paths: FfmpegPaths {
                ffmpeg: PathBuf::from("C:/x/ffmpeg.exe"),
                ffprobe: PathBuf::from("C:/x/ffprobe.exe"),
                source: tf_media::LocateSource::BundledSidecar,
            },
            caps: Box::new(caps()),
            cached: true,
            elapsed_ms: 3,
        }
    }

    #[test]
    fn probe_starts_idle_and_reports_probing() {
        let state = AppState::default();
        assert!(state.probe_state().probing());
        // 未就绪时给出可读提示，而不是 panic 或阻塞
        let err = state.require_toolchain().unwrap_err();
        assert!(err.to_string().contains("正在检测"));
    }

    #[test]
    fn begin_probe_is_idempotent() {
        let state = AppState::default();
        assert!(state.begin_probe(), "首次应允许启动探测");
        assert!(!state.begin_probe(), "探测中不应重复启动");
        assert!(state.probe_state().probing());
    }

    #[test]
    fn invalidate_forces_reprobe() {
        let state = AppState::default();
        assert!(state.begin_probe());
        *lock(&state.probe) = ready_state();
        assert!(state.probe_state().ready().is_some());
        state.invalidate_probe();
        assert!(state.probe_state().probing());
        assert!(state.begin_probe());
    }

    #[test]
    fn ready_state_serves_toolchain_and_failed_state_propagates_message() {
        let state = AppState::default();
        *lock(&state.probe) = ready_state();
        let (paths, caps) = state.require_toolchain().unwrap();
        assert!(paths.ffmpeg.ends_with("ffmpeg.exe"));
        assert!(caps.supports(tf_core::model::AudioFormat::Flac));

        *lock(&state.probe) = ProbeState::Failed("找不到 ffmpeg".into());
        let err = state.require_toolchain().unwrap_err();
        assert!(err.to_string().contains("找不到 ffmpeg"));
    }

    #[test]
    fn items_selection_and_covers_still_work() {
        let state = AppState::default();
        state.extend_items(vec![item("C:/a.flac"), item("C:/b.flac")]);
        assert_eq!(state.items().len(), 2);
        assert_eq!(state.select(&[]).len(), 2);
        let first = state.items()[0].id.clone();
        assert_eq!(state.select(&[first.clone()]).len(), 1);
        state.update_items(&[first.clone()], |i| i.tags.title = Some("新标题".into()));
        assert_eq!(state.item(&first).unwrap().tags.title.as_deref(), Some("新标题"));
        state.stage_cover(&first, CoverAction::Remove);
        assert!(matches!(state.take_cover(&first), Some(CoverAction::Remove)));
        state.remove_items(&[first]);
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
}
