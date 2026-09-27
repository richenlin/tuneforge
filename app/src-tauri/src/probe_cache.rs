//! FFmpeg 能力探测结果磁盘缓存。
//!
//! 首次启动必须 spawn `ffmpeg -version` 与 `ffmpeg -encoders`；随后同一份二进制
//! （大小 + 修改时间一致）不必重复探测，直接命中缓存 → 启动零子进程。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tf_media::{Capabilities, FfmpegPaths};

/// 单个可执行文件的指纹。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprint {
    /// 规范化路径。
    pub path: String,
    /// 文件大小（字节）。
    pub len: u64,
    /// 修改时间（Unix 毫秒；不可得时为 `None`）。
    pub modified_ms: Option<u128>,
}

impl FileFingerprint {
    /// 读取指纹；文件不存在时返回 `None`。
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis());
        Some(Self {
            path: path.to_string_lossy().to_string(),
            len: meta.len(),
            modified_ms,
        })
    }
}

/// 缓存条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCache {
    /// ffmpeg 指纹。
    pub ffmpeg: FileFingerprint,
    /// ffprobe 指纹。
    pub ffprobe: FileFingerprint,
    /// 探测结果。
    pub capabilities: Capabilities,
}

impl ProbeCache {
    /// 依据当前二进制生成条目。
    pub fn new(paths: &FfmpegPaths, capabilities: &Capabilities) -> Option<Self> {
        Some(Self {
            ffmpeg: FileFingerprint::of(&paths.ffmpeg)?,
            ffprobe: FileFingerprint::of(&paths.ffprobe)?,
            capabilities: capabilities.clone(),
        })
    }

    /// 当前二进制是否与缓存一致。
    pub fn matches(&self, paths: &FfmpegPaths) -> bool {
        let Some(ffmpeg) = FileFingerprint::of(&paths.ffmpeg) else {
            return false;
        };
        let Some(ffprobe) = FileFingerprint::of(&paths.ffprobe) else {
            return false;
        };
        ffmpeg == self.ffmpeg && ffprobe == self.ffprobe
    }
}

/// 读取缓存（文件缺失/损坏都当作未命中）。
pub fn load(path: &Path) -> Option<ProbeCache> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 写入缓存（失败不影响主流程）。
pub fn save(path: &Path, cache: &ProbeCache) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = fs::write(path, text);
    }
}

/// 依据缓存目录生成缓存文件路径。
pub fn cache_file(dir: &Path) -> PathBuf {
    dir.join("ffmpeg-capabilities.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn caps() -> Capabilities {
        Capabilities {
            version_line: "ffmpeg version 8.1".into(),
            configuration: "--enable-lgpl".into(),
            encoders: vec!["flac".into(), "libmp3lame".into()],
            raw_encoders: "A....D flac".into(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tuneforge-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_binary(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn roundtrip_hits_when_binaries_unchanged() {
        let dir = temp_dir("hit");
        let ffmpeg = fake_binary(&dir, "ffmpeg.exe", b"1234");
        let ffprobe = fake_binary(&dir, "ffprobe.exe", b"5678");
        let paths = FfmpegPaths {
            ffmpeg,
            ffprobe,
            source: tf_media::LocateSource::BundledSidecar,
        };
        let entry = ProbeCache::new(&paths, &caps()).unwrap();
        assert!(entry.matches(&paths));

        let file = cache_file(&dir);
        save(&file, &entry);
        let loaded = load(&file).unwrap();
        assert_eq!(loaded, entry);
        assert!(loaded.matches(&paths));
    }

    #[test]
    fn misses_when_binary_changes() {
        let dir = temp_dir("miss");
        let ffmpeg = fake_binary(&dir, "ffmpeg.exe", b"1234");
        let ffprobe = fake_binary(&dir, "ffprobe.exe", b"5678");
        let paths = FfmpegPaths {
            ffmpeg: ffmpeg.clone(),
            ffprobe,
            source: tf_media::LocateSource::BundledSidecar,
        };
        let entry = ProbeCache::new(&paths, &caps()).unwrap();
        // 二进制被替换（大小变化）→ 缓存不再匹配
        fs::write(&ffmpeg, b"1234567890").unwrap();
        assert!(!entry.matches(&paths));
    }

    #[test]
    fn missing_or_broken_cache_is_a_miss() {
        let dir = temp_dir("broken");
        let file = cache_file(&dir);
        assert!(load(&file).is_none());
        fs::write(&file, "{ not json").unwrap();
        assert!(load(&file).is_none());
    }

    #[test]
    fn missing_binary_yields_no_cache_entry() {
        let paths = FfmpegPaths {
            ffmpeg: PathBuf::from("C:/definitely/missing/ffmpeg.exe"),
            ffprobe: PathBuf::from("C:/definitely/missing/ffprobe.exe"),
            source: tf_media::LocateSource::Path,
        };
        assert!(ProbeCache::new(&paths, &caps()).is_none());
    }
}
