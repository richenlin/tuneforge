//! Tuneforge Tauri 后端：命令、状态与事件。
#![forbid(unsafe_code)]

pub mod commands;
pub mod dto;
pub mod probe_cache;
pub mod state;

use tauri::Manager;

use state::AppState;

/// 初始化日志（分级 + 标准错误输出；发布版可换成滚动文件）。
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("TUNEFORGE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tuneforge_lib=debug,tf_jobs=debug"));
    let _ = fmt().with_env_filter(filter).with_target(true).try_init();
}

/// 启动应用。
pub fn run() {
    init_tracing();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::ffmpeg_status,
            commands::set_ffmpeg_path,
            commands::scan_inputs,
            commands::list_items,
            commands::clear_items,
            commands::remove_items,
            commands::encode_options,
            commands::default_encode_spec,
            commands::sample_rate_options,
            commands::measure_loudness,
            commands::template_presets,
            commands::preview_rename,
            commands::update_tags,
            commands::replace_in_tags,
            commands::guess_tags,
            commands::cover_preview,
            commands::set_cover,
            commands::remove_cover,
            commands::export_cover,
            commands::start_job,
            commands::cancel_job,
            commands::job_report,
            commands::export_plan,
            commands::export_report,
            commands::open_in_explorer,
            commands::app_info,
            commands::preview_decode,
            commands::gain_preview,
            commands::conflict_policies,
            commands::default_normalize_params,
        ])
        .setup(|app| {
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "Tuneforge 已启动（非破坏性模式：只读源文件）"
            );
            // 关键：ffmpeg 探测（会 spawn 134 MB 的子进程，冷启动可能被 Defender 扫描）
            // 放到后台线程，窗口绘制与前端首帧不再等待它；完成后通过 ffmpeg:status 事件回推。
            let state = app.state::<AppState>();
            state.start_probe(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
