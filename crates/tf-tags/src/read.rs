//! 读取标签与封面（设计方案 §8）。

use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};
use tf_core::error::{Result, TfError};
use tf_core::model::{CoverArt, Tags};

/// 读取结果：标签 + 封面。
#[derive(Debug, Clone, PartialEq)]
pub struct TagSnapshot {
    /// 标准字段。
    pub tags: Tags,
    /// 封面列表（通常只有一张）。
    pub covers: Vec<CoverArt>,
    /// 容器实际使用的标签类型。
    pub tag_type: Option<String>,
}

/// 打开文件（统一的错误分类）。
pub(crate) fn open_tagged(path: &Path) -> Result<lofty::file::TaggedFile> {
    if !path.is_file() {
        return Err(TfError::Input(format!("文件不存在：{}", path.display())));
    }
    Probe::open(path)
        .map_err(|e| TfError::Tag(format!("无法打开 {}：{e}", path.display())))?
        .options(ParseOptions::new())
        .read()
        .map_err(|e| TfError::Tag(format!("解析标签失败（{}）：{e}", path.display())))
}

/// 把 lofty `Tag` 映射成领域 [`Tags`]（纯函数，便于单测）。
pub fn tags_from_lofty(tag: &Tag) -> Tags {
    Tags {
        title: tag.title().map(|v| v.to_string()),
        artist: tag.artist().map(|v| v.to_string()),
        album: tag.album().map(|v| v.to_string()),
        album_artist: tag.get_string(&ItemKey::AlbumArtist).map(|v| v.to_string()),
        track: tag.track(),
        disc: tag.disk(),
        year: tag.year().map(|v| v.to_string()),
        genre: tag.genre().map(|v| v.to_string()),
        comment: tag.comment().map(|v| v.to_string()),
    }
    .normalized()
}

/// 把领域 [`Tags`] 写入 lofty `Tag`（空值等于清除该字段）。
pub fn apply_tags_to_lofty(tag: &mut Tag, tags: &Tags) {
    let tags = tags.clone().normalized();

    match &tags.title {
        Some(v) => tag.set_title(v.clone()),
        None => tag.remove_title(),
    }
    match &tags.artist {
        Some(v) => tag.set_artist(v.clone()),
        None => tag.remove_artist(),
    }
    match &tags.album {
        Some(v) => tag.set_album(v.clone()),
        None => tag.remove_album(),
    }
    match &tags.genre {
        Some(v) => tag.set_genre(v.clone()),
        None => tag.remove_genre(),
    }
    match &tags.comment {
        Some(v) => tag.set_comment(v.clone()),
        None => tag.remove_comment(),
    }
    match tags.track {
        Some(v) => tag.set_track(v),
        None => tag.remove_track(),
    }
    match tags.disc {
        Some(v) => tag.set_disk(v),
        None => tag.remove_disk(),
    }
    match tags.year.as_deref().and_then(|v| v.parse::<u32>().ok()) {
        Some(v) => tag.set_year(v),
        None => tag.remove_year(),
    }
    match &tags.album_artist {
        Some(v) => {
            tag.insert_text(ItemKey::AlbumArtist, v.clone());
        }
        None => tag.remove_key(&ItemKey::AlbumArtist),
    }
}

/// 读取文件的标签。
pub fn read_tags(path: &Path) -> Result<Tags> {
    Ok(read_snapshot(path)?.tags)
}

/// 读取标签与封面。
pub fn read_snapshot(path: &Path) -> Result<TagSnapshot> {
    let tagged = open_tagged(path)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let Some(tag) = tag else {
        return Ok(TagSnapshot {
            tags: Tags::default(),
            covers: Vec::new(),
            tag_type: None,
        });
    };

    Ok(TagSnapshot {
        tags: tags_from_lofty(tag),
        covers: tag
            .pictures()
            .iter()
            .map(|p| CoverArt {
                mime: p
                    .mime_type()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "application/octet-stream".into()),
                data: p.data().to_vec(),
            })
            .collect(),
        tag_type: Some(format!("{:?}", tag.tag_type())),
    })
}

/// 读取标签与首张封面（列表用）。
pub fn read_tags_and_cover(path: &Path) -> Result<(Tags, Option<CoverArt>)> {
    let snapshot = read_snapshot(path)?;
    let cover = snapshot.covers.into_iter().next();
    Ok((snapshot.tags, cover))
}

/// 该文件是否已包含标签。
pub fn has_existing_tag(path: &Path) -> Result<bool> {
    let tagged = open_tagged(path)?;
    Ok(tagged.primary_tag().is_some() || tagged.first_tag().is_some())
}

/// 容器首选的标签类型（新建标签时用）。
pub fn primary_tag_type(path: &Path) -> Result<TagType> {
    let tagged = open_tagged(path)?;
    Ok(tagged.primary_tag_type())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_tags() -> Tags {
        Tags {
            title: Some("Yesterday".into()),
            artist: Some("The Beatles".into()),
            album: Some("Help!".into()),
            album_artist: Some("The Beatles".into()),
            track: Some(7),
            disc: Some(1),
            year: Some("1965".into()),
            genre: Some("Rock".into()),
            comment: Some("  ".into()),
        }
    }

    #[test]
    fn roundtrip_between_domain_tags_and_lofty_tag() {
        let mut tag = Tag::new(TagType::VorbisComments);
        apply_tags_to_lofty(&mut tag, &sample_tags());
        let back = tags_from_lofty(&tag);
        assert_eq!(back.title.as_deref(), Some("Yesterday"));
        assert_eq!(back.artist.as_deref(), Some("The Beatles"));
        assert_eq!(back.album.as_deref(), Some("Help!"));
        assert_eq!(back.album_artist.as_deref(), Some("The Beatles"));
        assert_eq!(back.track, Some(7));
        assert_eq!(back.disc, Some(1));
        assert_eq!(back.year.as_deref(), Some("1965"));
        assert_eq!(back.genre.as_deref(), Some("Rock"));
        // 空白评论视为无值
        assert_eq!(back.comment, None);
    }

    /// 「清除其他标签」：一次清掉除「艺术家 / 标题」以外的全部字段（内存视图与底层标签都要干净）。
    #[test]
    fn clearing_other_fields_leaves_only_artist_and_title() {
        let full = Tags {
            comment: Some("note".into()),
            ..sample_tags()
        };
        let keep = Tags {
            title: full.title.clone(),
            artist: full.artist.clone(),
            ..Tags::default()
        };

        for tag_type in [TagType::Id3v2, TagType::VorbisComments] {
            let mut tag = Tag::new(tag_type);
            apply_tags_to_lofty(&mut tag, &full);
            assert_eq!(
                tags_from_lofty(&tag).comment.as_deref(),
                Some("note"),
                "清除前备注应可读: {tag_type:?}"
            );

            apply_tags_to_lofty(&mut tag, &keep);
            let back = tags_from_lofty(&tag);
            assert_eq!(back, keep.clone().normalized(), "仅保留艺术家 / 标题: {tag_type:?}");

            // 直接查底层标签，确认不是只清了内存视图
            assert!(tag.get_string(&ItemKey::Comment).is_none(), "{tag_type:?}");
            assert!(tag.get_string(&ItemKey::AlbumArtist).is_none(), "{tag_type:?}");
            assert!(tag.album().is_none(), "{tag_type:?}");
            assert!(tag.genre().is_none(), "{tag_type:?}");
            assert!(tag.track().is_none(), "{tag_type:?}");
            assert!(tag.disk().is_none(), "{tag_type:?}");
            assert!(tag.year().is_none(), "{tag_type:?}");
        }
    }

    /// 外部工具（如 ffmpeg）写出的小写 Vorbis 键也必须在读取时被识别，否则清除不干净。
    #[test]
    fn lowercase_vorbis_keys_are_recognized() {
        assert_eq!(
            ItemKey::from_key(TagType::VorbisComments, "comment"),
            ItemKey::Comment
        );
        assert_eq!(
            ItemKey::from_key(TagType::VorbisComments, "albumartist"),
            ItemKey::AlbumArtist
        );
    }

    #[test]
    fn empty_values_clear_existing_fields() {
        let mut tag = Tag::new(TagType::Id3v2);
        apply_tags_to_lofty(&mut tag, &sample_tags());
        apply_tags_to_lofty(&mut tag, &Tags::default());
        let back = tags_from_lofty(&tag);
        assert_eq!(back, Tags::default());
        assert!(tag.get_string(&ItemKey::AlbumArtist).is_none());
    }

    #[test]
    fn non_numeric_year_is_dropped_not_fatal() {
        let mut tag = Tag::new(TagType::Id3v2);
        let tags = Tags {
            year: Some("1965-1970".into()),
            ..Tags::default()
        };
        apply_tags_to_lofty(&mut tag, &tags);
        assert!(tags_from_lofty(&tag).year.is_none());
    }

    #[test]
    fn missing_file_is_input_error() {
        let err = read_tags(&PathBuf::from("C:/definitely/not/here.flac")).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }

    #[test]
    fn unsupported_or_garbage_file_is_tag_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.flac");
        std::fs::write(&path, b"not really a flac").unwrap();
        let err = read_tags(&path).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Tag);
    }
}
