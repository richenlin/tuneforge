//! 输出目录、冲突策略与原子替换（设计方案 §10.1–§10.3）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tf_core::error::{Result, TfError};

/// 同名文件处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// 跳过已存在文件（默认，D9）。
    #[default]
    Skip,
    /// 覆盖已存在文件。
    Overwrite,
    /// 自动重命名，追加 ` (1)`、` (2)`…
    Rename,
}

impl ConflictPolicy {
    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            ConflictPolicy::Skip => "跳过同名的已存在文件",
            ConflictPolicy::Overwrite => "覆盖已存在的文件",
            ConflictPolicy::Rename => "自动重命名（追加序号）",
        }
    }
}

/// 单次输出的决策。
#[derive(Debug, Clone, PartialEq)]
pub enum OutputDecision {
    /// 需要写盘：`temp` 是临时文件，提交时 rename 为 `final_path`。
    Write {
        /// 临时文件路径。
        temp: PathBuf,
        /// 最终路径。
        final_path: PathBuf,
    },
    /// 跳过（并给出原因）。
    Skip {
        /// 原因说明。
        reason: String,
    },
}

/// 校验输出目录（§10.1：不提供原地修改）。
pub fn validate_output_dir(output_dir: &Path, sources: &[PathBuf]) -> Result<()> {
    if output_dir.as_os_str().is_empty() {
        return Err(TfError::Input("请选择输出目录".into()));
    }
    let canonical_out = canonicalize_loose(output_dir);
    for src in sources {
        let parent = src.parent().unwrap_or(Path::new(""));
        if canonicalize_loose(parent) == canonical_out {
            return Err(TfError::Input(format!(
                "输出目录不能与源文件所在目录相同（{}）——为保证非破坏性，请选择新文件夹",
                output_dir.display()
            )));
        }
    }
    Ok(())
}

fn canonicalize_loose(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// 生成临时文件路径。
pub fn temp_path(dir: &Path, extension: &str) -> PathBuf {
    let id = uuid::Uuid::new_v4();
    let ext = extension.trim_start_matches('.');
    if ext.is_empty() {
        dir.join(format!(".tf-tmp-{id}"))
    } else {
        dir.join(format!(".tf-tmp-{id}.{ext}"))
    }
}

/// 解析输出路径（含冲突策略）。
pub struct OutputResolver {
    dir: PathBuf,
    policy: ConflictPolicy,
    /// 本次运行已占用的目标路径（并发安全），避免两个任务写到同一名字。
    reserved: Mutex<HashSet<PathBuf>>,
}

impl OutputResolver {
    /// 新建（会创建输出目录）。
    pub fn new(dir: impl Into<PathBuf>, policy: ConflictPolicy) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(OutputResolver {
            dir,
            policy,
            reserved: Mutex::new(HashSet::new()),
        })
    }

    /// 输出目录。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 冲突策略。
    pub fn policy(&self) -> ConflictPolicy {
        self.policy
    }

    /// 解析目标文件名。
    pub fn resolve(&self, file_name: &str) -> Result<OutputDecision> {
        let safe_name = sanitize_file_name(file_name);
        let (stem, ext) = split_name(&safe_name);
        let dotted_ext = if ext.is_empty() {
            String::new()
        } else {
            format!(".{ext}")
        };
        let mut candidate = self.dir.join(&safe_name);
        let mut index = 1u32;

        loop {
            let taken_by_peer = {
                let guard = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
                guard.contains(&candidate)
            };
            let exists = candidate.exists();

            if !taken_by_peer && !exists {
                self.reserve(&candidate);
                return Ok(OutputDecision::Write {
                    temp: temp_path(&self.dir, ext),
                    final_path: candidate,
                });
            }

            match self.policy {
                ConflictPolicy::Skip => {
                    return Ok(OutputDecision::Skip {
                        reason: if taken_by_peer {
                            format!("同名目标已在本次任务中占用：{}", candidate.display())
                        } else {
                            format!("输出目录已存在同名文件：{}", candidate.display())
                        },
                    });
                }
                ConflictPolicy::Overwrite => {
                    self.reserve(&candidate);
                    return Ok(OutputDecision::Write {
                        temp: temp_path(&self.dir, ext),
                        final_path: candidate,
                    });
                }
                ConflictPolicy::Rename => {
                    index += 1;
                    if index > 9_999 {
                        return Err(TfError::Io(format!(
                            "无法为 {safe_name} 找到可用的输出名（已尝试 9999 次）"
                        )));
                    }
                    let name = format!("{stem} ({}){dotted_ext}", index - 1);
                    candidate = self.dir.join(name);
                }
            }
        }
    }

    fn reserve(&self, path: &Path) {
        let mut guard = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(path.to_path_buf());
    }
}

fn split_name(name: &str) -> (String, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext),
        _ => (name.to_string(), ""),
    }
}

/// 去掉路径分隔符等危险字符（防御性，正常流程里名字已由 tf-core 合法化）。
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() {
        "未命名".into()
    } else {
        trimmed.to_string()
    }
}

/// 临时文件守卫：未提交时 `Drop` 会删除临时文件（失败/取消清理）。
#[derive(Debug)]
pub struct TempGuard {
    path: PathBuf,
    committed: bool,
}

impl TempGuard {
    /// 接管一个临时路径。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        TempGuard {
            path: path.into(),
            committed: false,
        }
    }

    /// 临时文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 原子提交：把临时文件 rename 为最终路径。
    ///
    /// 优先**单次 rename**（Windows 上 `MoveFileEx(REPLACE_EXISTING)` 可直接覆盖），
    /// 避免“先删后改名”在崩溃/断电窗口内丢失旧输出、并让最终路径短暂不存在。
    /// 仅在 rename 失败且目标已存在时（只读/被占用等）才回退到先删后改名。
    pub fn commit(mut self, final_path: &Path) -> Result<PathBuf> {
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::rename(&self.path, final_path) {
            Ok(()) => {}
            Err(first) => {
                if !final_path.exists() {
                    return Err(TfError::Io(format!(
                        "提交输出失败（{} → {}）：{first}",
                        self.path.display(),
                        final_path.display()
                    )));
                }
                std::fs::remove_file(final_path)
                    .map_err(|e| TfError::Io(format!("无法覆盖 {}：{e}", final_path.display())))?;
                std::fs::rename(&self.path, final_path).map_err(|e| {
                    TfError::Io(format!(
                        "提交输出失败（{} → {}）：{e}（首次尝试：{first}）",
                        self.path.display(),
                        final_path.display()
                    ))
                })?;
            }
        }
        self.committed = true;
        Ok(final_path.to_path_buf())
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.committed && self.path.exists() {
            if let Err(e) = std::fs::remove_file(&self.path) {
                tracing::warn!(path = %self.path.display(), error = %e, "清理临时文件失败");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_paths_are_hidden_and_unique() {
        let dir = Path::new("C:/out");
        let a = temp_path(dir, "flac");
        let b = temp_path(dir, ".flac");
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains(".tf-tmp-"));
        assert!(a.to_string_lossy().ends_with(".flac"));
        let no_ext = temp_path(dir, "");
        assert!(no_ext.to_string_lossy().contains(".tf-tmp-"));
    }

    #[test]
    fn skip_policy_reports_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.flac"), b"x").unwrap();
        let resolver = OutputResolver::new(dir.path(), ConflictPolicy::Skip).unwrap();
        let decision = resolver.resolve("a.flac").unwrap();
        match decision {
            OutputDecision::Skip { reason } => assert!(reason.contains("已存在同名文件")),
            other => panic!("期望跳过，得到 {other:?}"),
        }
    }

    #[test]
    fn overwrite_policy_targets_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.flac"), b"x").unwrap();
        let resolver = OutputResolver::new(dir.path(), ConflictPolicy::Overwrite).unwrap();
        match resolver.resolve("a.flac").unwrap() {
            OutputDecision::Write { final_path, temp } => {
                assert_eq!(final_path, dir.path().join("a.flac"));
                assert!(temp.to_string_lossy().contains(".tf-tmp-"));
            }
            other => panic!("期望写入，得到 {other:?}"),
        }
    }

    #[test]
    fn rename_policy_appends_index_and_respects_batch_reservations() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a (1).flac"), b"x").unwrap();
        let resolver = OutputResolver::new(dir.path(), ConflictPolicy::Rename).unwrap();

        let first = resolver.resolve("a.flac").unwrap();
        match first {
            OutputDecision::Write { final_path, .. } => {
                assert_eq!(final_path, dir.path().join("a.flac"))
            }
            other => panic!("{other:?}"),
        }
        // 第二次同名（批内重复）应拿到 (2)，因为 (1) 已被磁盘占用
        match resolver.resolve("a.flac").unwrap() {
            OutputDecision::Write { final_path, .. } => {
                assert_eq!(final_path, dir.path().join("a (2).flac"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn skip_policy_also_guards_in_batch_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = OutputResolver::new(dir.path(), ConflictPolicy::Skip).unwrap();
        assert!(matches!(
            resolver.resolve("same.flac").unwrap(),
            OutputDecision::Write { .. }
        ));
        match resolver.resolve("same.flac").unwrap() {
            OutputDecision::Skip { reason } => assert!(reason.contains("本次任务")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn temp_guard_removes_file_when_not_committed() {
        let dir = tempfile::tempdir().unwrap();
        let temp = temp_path(dir.path(), "flac");
        std::fs::write(&temp, b"partial").unwrap();
        {
            let _guard = TempGuard::new(&temp);
        }
        assert!(!temp.exists(), "未提交的临时文件应被清理");
    }

    #[test]
    fn temp_guard_commits_atomically_and_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("out.flac");
        std::fs::write(&final_path, b"old").unwrap();
        let temp = temp_path(dir.path(), "flac");
        std::fs::write(&temp, b"new").unwrap();
        let committed = TempGuard::new(&temp).commit(&final_path).unwrap();
        assert_eq!(committed, final_path);
        assert_eq!(std::fs::read(&final_path).unwrap(), b"new");
        assert!(!temp.exists());
    }

    #[test]
    fn output_dir_equal_to_source_dir_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.flac");
        std::fs::write(&src, b"x").unwrap();
        let err = validate_output_dir(dir.path(), &[src]).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
        let out = dir.path().join("out");
        assert!(validate_output_dir(&out, &[dir.path().join("a.flac")]).is_ok());
        assert!(validate_output_dir(Path::new(""), &[]).is_err());
    }

    #[test]
    fn dangerous_names_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = OutputResolver::new(dir.path(), ConflictPolicy::Skip).unwrap();
        match resolver.resolve("bad/name:?.flac").unwrap() {
            OutputDecision::Write { final_path, .. } => {
                let name = final_path.file_name().unwrap().to_string_lossy().to_string();
                assert!(!name.contains('/'));
                assert!(!name.contains(':'));
                assert!(name.ends_with(".flac"));
            }
            other => panic!("{other:?}"),
        }
        match resolver.resolve("   ").unwrap() {
            OutputDecision::Write { final_path, .. } => {
                assert!(final_path.to_string_lossy().contains("未命名"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn policy_labels_are_chinese() {
        assert!(ConflictPolicy::default() == ConflictPolicy::Skip);
        assert!(ConflictPolicy::Overwrite.label().contains("覆盖"));
    }
}
