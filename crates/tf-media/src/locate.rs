//! 定位捆绑的 ffmpeg / ffprobe（设计方案 §11）。

use std::path::{Path, PathBuf};

use serde::Serialize;
use tf_core::error::{Result, TfError};

/// 可执行文件来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocateSource {
    /// 用户显式指定（`--ffmpeg-path` / 设置项）。
    Override,
    /// 随包分发的 sidecar 目录。
    BundledSidecar,
    /// 与主程序同目录。
    ExecutableDir,
    /// 系统 PATH。
    Path,
}

impl LocateSource {
    /// 中文说明（UI 展示）。
    pub fn label(self) -> &'static str {
        match self {
            LocateSource::Override => "用户指定",
            LocateSource::BundledSidecar => "随包分发",
            LocateSource::ExecutableDir => "程序目录",
            LocateSource::Path => "系统 PATH",
        }
    }
}

/// 一组 ffmpeg / ffprobe 路径。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FfmpegPaths {
    /// ffmpeg 可执行文件。
    pub ffmpeg: PathBuf,
    /// ffprobe 可执行文件。
    pub ffprobe: PathBuf,
    /// 来源。
    pub source: LocateSource,
}

/// 可执行文件后缀（Windows 需要 .exe）。
pub fn exe_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else {
        ""
    }
}

/// sidecar 目录名（Tauri `externalBin` 约定 `<name>-<target-triple>`）。
pub fn sidecar_dir_names() -> Vec<String> {
    let triple = target_triple();
    vec![format!("binaries/{triple}"), "binaries".to_string(), triple]
}

/// 常见 target triple 文本（用于拼接 sidecar 目录）。
pub fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "windows" => format!("{arch}-pc-windows-msvc"),
        "macos" => format!("{arch}-apple-darwin"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        other => format!("{arch}-unknown-{other}"),
    }
}

/// 在候选目录中查找可执行文件。
pub fn find_in_dirs(dirs: &[PathBuf]) -> Option<FfmpegPaths> {
    for dir in dirs {
        let ffmpeg = dir.join(format!("ffmpeg{}", exe_suffix()));
        let ffprobe = dir.join(format!("ffprobe{}", exe_suffix()));
        if matches_ffmpeg(&ffmpeg) && matches_ffmpeg(&ffprobe) {
            return Some(FfmpegPaths {
                ffmpeg,
                ffprobe,
                source: LocateSource::BundledSidecar,
            });
        }
    }
    None
}

fn matches_ffmpeg(path: &Path) -> bool {
    path.is_file() && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

/// 由主程序目录 / 资源目录推导 sidecar 候选目录（顺序即优先级）。
pub fn candidate_dirs(exe_dir: Option<&Path>, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(res) = resource_dir {
        for name in sidecar_dir_names() {
            dirs.push(res.join(&name));
        }
        dirs.push(res.join("binaries"));
        dirs.push(res.to_path_buf());
    }
    if let Some(exe) = exe_dir {
        for name in sidecar_dir_names() {
            dirs.push(exe.join(&name));
        }
        dirs.push(exe.join("binaries"));
        dirs.push(exe.to_path_buf());
    }
    dirs
}

/// 定位 ffmpeg / ffprobe。
///
/// 顺序：用户指定 → sidecar 候选目录 → PATH（见设计方案 §11）。
pub fn discover(override_dir: Option<&Path>, candidates: &[PathBuf]) -> Result<FfmpegPaths> {
    discover_opts(override_dir, candidates, true)
}

/// 同 [`discover`]，但可关闭 PATH 回退（测试/受控环境下保证结果可预测）。
pub fn discover_opts(
    override_dir: Option<&Path>,
    candidates: &[PathBuf],
    allow_path_fallback: bool,
) -> Result<FfmpegPaths> {
    if let Some(o) = override_dir {
        let dir_candidate = if o.is_dir() {
            Some(o.to_path_buf())
        } else if o.is_file() {
            o.parent().map(|p| p.to_path_buf())
        } else {
            None
        };
        if let Some(dir) = dir_candidate {
            let ffmpeg = dir.join(format!("ffmpeg{}", exe_suffix()));
            let ffprobe = dir.join(format!("ffprobe{}", exe_suffix()));
            if matches_ffmpeg(&ffmpeg) {
                return Ok(FfmpegPaths {
                    ffmpeg,
                    ffprobe,
                    source: LocateSource::Override,
                });
            }
        }
    }

    if let Some(paths) = find_in_dirs(candidates) {
        return Ok(paths);
    }

    // PATH 回退
    let ffmpeg = PathBuf::from(format!("ffmpeg{}", exe_suffix()));
    let ffprobe = PathBuf::from(format!("ffprobe{}", exe_suffix()));
    if allow_path_fallback && run_version(&ffmpeg).is_ok() {
        return Ok(FfmpegPaths {
            ffmpeg,
            ffprobe,
            source: LocateSource::Path,
        });
    }

    Err(TfError::Unsupported(format!(
        "未找到 ffmpeg/ffprobe。请把 ffmpeg{} 与 ffprobe{} 放到 sidecar 目录（{}），\
         或在设置中指定 ffmpeg 路径。",
        exe_suffix(),
        exe_suffix(),
        sidecar_dir_names().join(" / ")
    )))
}

/// 运行 `<prog> -version` 并返回输出。
pub fn run_version(prog: &Path) -> Result<String> {
    let output = crate::process::command(prog)
        .arg("-version")
        .output()
        .map_err(|e| TfError::Unsupported(format!("无法执行 {}：{e}", prog.display())))?;
    if !output.status.success() {
        return Err(TfError::Unsupported(format!(
            "{} -version 返回非零状态：{}",
            prog.display(),
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

impl FfmpegPaths {
    /// 两个可执行文件是否都可用。
    pub fn available(&self) -> bool {
        run_version(&self.ffmpeg).is_ok() && run_version(&self.ffprobe).is_ok()
    }

    /// `ffmpeg -version` 的第一行。
    pub fn version_line(&self) -> Result<String> {
        let out = run_version(&self.ffmpeg)?;
        Ok(out.lines().next().unwrap_or_default().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_names_include_triple_and_plain() {
        let names = sidecar_dir_names();
        assert!(names.iter().any(|n| n == "binaries"));
        assert!(names.iter().any(|n| n.contains(&target_triple())));
    }

    #[test]
    fn find_in_dirs_requires_both_binaries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("ffmpeg{}", exe_suffix())), b"x").unwrap();
        // 只有 ffmpeg，没有 ffprobe → 找不到
        assert!(find_in_dirs(&[dir.path().to_path_buf()]).is_none());
        std::fs::write(dir.path().join(format!("ffprobe{}", exe_suffix())), b"x").unwrap();
        let found = find_in_dirs(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(found.source, LocateSource::BundledSidecar);
        assert!(found.ffmpeg.ends_with(format!("ffmpeg{}", exe_suffix())));
    }

    #[test]
    fn empty_files_are_not_treated_as_binaries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("ffmpeg{}", exe_suffix())), b"").unwrap();
        std::fs::write(dir.path().join(format!("ffprobe{}", exe_suffix())), b"").unwrap();
        assert!(find_in_dirs(&[dir.path().to_path_buf()]).is_none());
    }

    #[test]
    fn candidate_dirs_order_prefers_resources() {
        let dirs = candidate_dirs(Some(Path::new("C:/app")), Some(Path::new("C:/app/res")));
        assert!(dirs[0].starts_with("C:/app/res"));
        assert!(dirs.last().unwrap().starts_with("C:/app"));
    }

    #[test]
    fn missing_binaries_produce_actionable_error() {
        // 关闭 PATH 回退，避免开发机/CI 装了 ffmpeg 时结果飘
        let err = discover_opts(None, &[PathBuf::from("C:/definitely/missing")], false).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Unsupported);
        assert!(err.to_string().contains("ffmpeg"));
    }

    #[test]
    fn override_directory_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("ffmpeg{}", exe_suffix())), b"x").unwrap();
        let found = discover(Some(dir.path()), &[]).unwrap();
        assert_eq!(found.source, LocateSource::Override);
    }
}
