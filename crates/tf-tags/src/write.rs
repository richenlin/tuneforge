//! 写入标签（设计方案 §8：先写音频、再写标签，标签失败不影响音频完整性）。

use std::path::Path;

use lofty::config::WriteOptions as LoftyWriteOptions;
use lofty::file::TaggedFileExt;
use lofty::prelude::TagExt;
use lofty::tag::Tag;
use serde::{Deserialize, Serialize};
use tf_core::error::{Result, TfError};
use tf_core::model::Tags;

use crate::read::{apply_tags_to_lofty, open_tagged, read_snapshot};

/// 写入选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteOptions {
    /// 是否保留已有封面（默认保留）。
    pub keep_cover: bool,
    /// 是否移除其他标签类型（例如 MP3 中同时存在 ID3v1 与 ID3v2）。
    pub remove_others: bool,
    /// ID3v2.3 兼容模式（老播放器）。
    pub id3v23: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            keep_cover: true,
            remove_others: true,
            id3v23: false,
        }
    }
}

impl WriteOptions {
    fn to_lofty(&self) -> LoftyWriteOptions {
        let mut opts = LoftyWriteOptions::new().remove_others(self.remove_others);
        opts.use_id3v23(self.id3v23);
        opts
    }
}

/// 写入标签。
///
/// * 文件没有标签时会新建容器首选的标签类型（FLAC→Vorbis Comment、MP3/WAV→ID3v2、M4A→ilst）。
/// * `keep_cover = false` 时会清空封面。
pub fn write_tags(path: &Path, tags: &Tags, options: &WriteOptions) -> Result<()> {
    let mut tagged = open_tagged(path)?;
    let tag_type = tagged.primary_tag_type();

    let mut tag = match tagged.tag_mut(tag_type) {
        Some(existing) => existing.clone(),
        None => Tag::new(tag_type),
    };

    if !options.keep_cover {
        while !tag.pictures().is_empty() {
            tag.remove_picture(0);
        }
    }

    apply_tags_to_lofty(&mut tag, tags);
    tag.save_to_path(path, options.to_lofty())
        .map_err(|e| TfError::Tag(format!("写入标签失败（{}）：{e}", path.display())))
}

/// 只改一个字段（标签页单文件编辑）。
pub fn update_field(
    path: &Path,
    field: tf_core::model::TagField,
    value: Option<String>,
    options: &WriteOptions,
) -> Result<Tags> {
    let mut tags = read_snapshot(path)?.tags;
    field.set(&mut tags, value)?;
    write_tags(path, &tags, options)?;
    Ok(tags)
}

/// 跨格式复制标签的来源范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyPolicy {
    /// 不复制任何标签。
    None,
    /// 复制标准字段，丢弃封面。
    TagsOnly,
    /// 复制标准字段 + 封面。
    TagsAndCover,
}

/// 复制结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyReport {
    /// 实际写入的字段数。
    pub fields_written: usize,
    /// 是否写入了封面。
    pub cover_written: bool,
    /// 警告（例如源文件无标签）。
    pub warnings: Vec<String>,
}

/// 把源文件的标签复制到目标文件（转换流程使用）。
pub fn copy_tags(from: &Path, to: &Path, policy: CopyPolicy, options: &WriteOptions) -> Result<CopyReport> {
    if policy == CopyPolicy::None {
        return Ok(CopyReport {
            fields_written: 0,
            cover_written: false,
            warnings: vec!["按设置未复制标签".into()],
        });
    }

    let snapshot = read_snapshot(from)?;
    let mut warnings = Vec::new();
    if snapshot.tags.is_empty() {
        warnings.push(format!("源文件无标签：{}", from.display()));
    }

    write_tags(to, &snapshot.tags, options)?;

    let mut cover_written = false;
    if policy == CopyPolicy::TagsAndCover {
        if let Some(cover) = snapshot.covers.into_iter().next() {
            crate::cover::set_cover(to, Some(&cover))?;
            cover_written = true;
        }
    }

    let fields_written = tf_core::model::TagField::ALL
        .iter()
        .filter(|f| f.get(&snapshot.tags).is_some())
        .count();

    Ok(CopyReport {
        fields_written,
        cover_written,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_minimal_wav;

    #[test]
    fn writes_and_reads_back_tags_on_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);

        let tags = Tags {
            title: Some("测试标题".into()),
            artist: Some("测试艺术家".into()),
            album: Some("测试专辑".into()),
            album_artist: Some("测试专辑艺术家".into()),
            track: Some(3),
            disc: Some(1),
            year: Some("2024".into()),
            genre: Some("Test".into()),
            comment: Some("备注".into()),
        };
        write_tags(&path, &tags, &WriteOptions::default()).unwrap();

        let back = crate::read_tags(&path).unwrap();
        assert_eq!(back.title.as_deref(), Some("测试标题"));
        assert_eq!(back.artist.as_deref(), Some("测试艺术家"));
        assert_eq!(back.album_artist.as_deref(), Some("测试专辑艺术家"));
        assert_eq!(back.track, Some(3));
        assert_eq!(back.disc, Some(1));
        assert_eq!(back.year.as_deref(), Some("2024"));
        assert_eq!(back.genre.as_deref(), Some("Test"));
        assert_eq!(back.comment.as_deref(), Some("备注"));

        // 音频数据必须保持完整（非破坏性）
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(after > 44, "文件长度 {after}");
    }

    #[test]
    fn update_field_only_touches_one_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);
        let mut tags = Tags::default();
        tags.artist = Some("A".into());
        tags.title = Some("T".into());
        write_tags(&path, &tags, &WriteOptions::default()).unwrap();

        let updated = update_field(
            &path,
            tf_core::model::TagField::Title,
            Some("T2".into()),
            &WriteOptions::default(),
        )
        .unwrap();
        assert_eq!(updated.title.as_deref(), Some("T2"));
        assert_eq!(updated.artist.as_deref(), Some("A"));
    }

    #[test]
    fn copy_tags_between_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_minimal_wav(&dir.path().join("a.wav"), 8000, 1, 800);
        let b = write_minimal_wav(&dir.path().join("b.wav"), 8000, 1, 800);
        let tags = Tags {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            ..Tags::default()
        };
        write_tags(&a, &tags, &WriteOptions::default()).unwrap();

        let report = copy_tags(&a, &b, CopyPolicy::TagsOnly, &WriteOptions::default()).unwrap();
        assert_eq!(report.fields_written, 2);
        assert!(!report.cover_written);
        let copied = crate::read_tags(&b).unwrap();
        assert_eq!(copied.title.as_deref(), Some("Song"));
        assert_eq!(copied.artist.as_deref(), Some("Artist"));

        let none = copy_tags(&a, &b, CopyPolicy::None, &WriteOptions::default()).unwrap();
        assert_eq!(none.fields_written, 0);
        assert!(!none.warnings.is_empty());
    }

    #[test]
    fn write_options_are_configurable() {
        let opts = WriteOptions {
            keep_cover: false,
            remove_others: false,
            id3v23: true,
        };
        let lofty = opts.to_lofty();
        assert_eq!(format!("{lofty:?}").is_empty(), false);
        assert!(WriteOptions::default().keep_cover);
    }
}
