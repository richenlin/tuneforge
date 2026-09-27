//! 输入扫描（设计方案 §4.3 第一步）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tf_core::error::{Result, TfError};
use tf_core::model::AudioFormat;

/// 可识别的音频扩展名（小写，不含点）。
pub fn audio_extensions() -> Vec<&'static str> {
    let mut set: BTreeSet<&'static str> = BTreeSet::new();
    for f in AudioFormat::ALL {
        set.insert(f.extension());
    }
    // 常见别名
    for extra in ["wave", "aif", "aifc", "m4b", "mp2", "oga", "dsf", "dff"] {
        set.insert(extra);
    }
    set.into_iter().collect()
}

/// 该路径是否是支持的音频文件（按扩展名）。
pub fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            audio_extensions().contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// 扫描输入（文件或目录），返回去重、排序后的音频文件列表。
///
/// * 目录默认不递归；`recursive = true` 时递归子目录。
/// * 单文件若是支持的音频格式也会被接受。
/// * 符号链接不跟随（避免环）。
pub fn scan_inputs(paths: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut found: BTreeSet<PathBuf> = BTreeSet::new();

    for path in paths {
        if path.is_file() {
            if is_supported_audio(path) {
                found.insert(normalize(path));
            } else {
                tracing::debug!(path = %path.display(), "跳过不支持的扩展名");
            }
            continue;
        }
        if !path.is_dir() {
            return Err(TfError::Input(format!("路径不存在：{}", path.display())));
        }

        let walker = walkdir::WalkDir::new(path)
            .max_depth(if recursive { usize::MAX } else { 1 })
            .follow_links(false);
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            if is_supported_audio(entry.path()) {
                found.insert(normalize(entry.path()));
            }
        }
    }

    Ok(found.into_iter().collect())
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_list_covers_design_formats() {
        let exts = audio_extensions();
        for e in [
            "flac", "wav", "aiff", "m4a", "mp3", "ogg", "opus", "ape", "dsf", "dff", "wv",
        ] {
            assert!(exts.contains(&e), "缺少扩展名 {e}");
        }
    }

    #[test]
    fn supported_audio_is_case_insensitive() {
        assert!(is_supported_audio(Path::new("a.FLAC")));
        assert!(is_supported_audio(Path::new("a.DsF")));
        assert!(!is_supported_audio(Path::new("a.txt")));
        assert!(!is_supported_audio(Path::new("noext")));
    }

    #[test]
    fn scan_directory_respects_recursive_flag() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("cd1");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(root.path().join("a.flac"), b"x").unwrap();
        std::fs::write(root.path().join("b.mp3"), b"x").unwrap();
        std::fs::write(root.path().join("cover.jpg"), b"x").unwrap();
        std::fs::write(sub.join("c.flac"), b"x").unwrap();

        let flat = scan_inputs(&[root.path().to_path_buf()], false).unwrap();
        assert_eq!(flat.len(), 2, "{flat:?}");

        let deep = scan_inputs(&[root.path().to_path_buf()], true).unwrap();
        assert_eq!(deep.len(), 3, "{deep:?}");
    }

    #[test]
    fn explicit_files_are_accepted_and_deduplicated() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("song.flac");
        std::fs::write(&file, b"x").unwrap();
        let out = scan_inputs(&[file.clone(), file.clone()], false).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn unsupported_single_file_is_ignored_and_missing_path_errors() {
        let root = tempfile::tempdir().unwrap();
        let txt = root.path().join("notes.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert!(scan_inputs(&[txt], false).unwrap().is_empty());

        let err = scan_inputs(&[root.path().join("nope")], false).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }
}
