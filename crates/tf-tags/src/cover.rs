//! 封面：读取 / 嵌入 / 替换 / 删除 / 导出（设计方案 §6.5 / R4）。

use std::path::{Path, PathBuf};

use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::TagExt;
use lofty::tag::Tag;
use tf_core::error::{Result, TfError};
use tf_core::model::CoverArt;

use crate::read::{open_tagged, read_snapshot};

/// 读取全部封面。
pub fn read_covers(path: &Path) -> Result<Vec<CoverArt>> {
    Ok(read_snapshot(path)?.covers)
}

/// 读取首张封面。
pub fn read_first_cover(path: &Path) -> Result<Option<CoverArt>> {
    Ok(read_covers(path)?.into_iter().next())
}

fn mime_of(cover: &CoverArt) -> MimeType {
    MimeType::from_str(&cover.mime)
}

/// 嵌入或替换封面（`None` 等于删除全部封面）。
///
/// 用默认标签选项；写入 MP3/WAV/AIFF 时请改用 [`set_cover_with`]，
/// 否则会按 lofty 默认把已有标签改写成 ID3v2.4（老播放器兼容性变差）。
pub fn set_cover(path: &Path, cover: Option<&CoverArt>) -> Result<()> {
    set_cover_with(path, cover, &crate::write::WriteOptions::default())
}

/// 嵌入或替换封面，沿用调用方的标签写入选项。
pub fn set_cover_with(
    path: &Path,
    cover: Option<&CoverArt>,
    options: &crate::write::WriteOptions,
) -> Result<()> {
    let mut tagged = open_tagged(path)?;
    let tag_type = tagged.primary_tag_type();
    let mut tag = match tagged.tag_mut(tag_type) {
        Some(existing) => existing.clone(),
        None => Tag::new(tag_type),
    };

    // 清掉已有封面（替换语义）
    while !tag.pictures().is_empty() {
        tag.remove_picture(0);
    }

    if let Some(cover) = cover {
        if cover.is_empty() {
            return Err(TfError::Input("封面数据为空".into()));
        }
        let picture = Picture::new_unchecked(
            PictureType::CoverFront,
            Some(mime_of(cover)),
            // 必须给非空描述：ID3v2.3 里空描述会被写成 UTF-16 空串（无 BOM），
            // lofty 自己都读不回来（"UTF-16 string has an invalid byte order mark"）。
            Some("Cover".to_string()),
            cover.data.clone(),
        );
        tag.push_picture(picture);
    }

    tag.save_to_path(path, options.to_lofty())
        .map_err(|e| TfError::Tag(format!("写入封面失败（{}）：{e}", path.display())))
}

/// 删除全部封面。
pub fn remove_covers(path: &Path) -> Result<()> {
    set_cover(path, None)
}

/// 导出第 `index` 张封面到目标路径；返回实际写入的路径。
pub fn export_cover(path: &Path, index: usize, out_dir: &Path) -> Result<PathBuf> {
    let covers = read_covers(path)?;
    let cover = covers.get(index).ok_or_else(|| {
        TfError::Input(format!(
            "{} 没有第 {} 张封面（共 {} 张）",
            path.display(),
            index + 1,
            covers.len()
        ))
    })?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cover")
        .to_string();
    let ext = cover
        .mime
        .rsplit('/')
        .next()
        .filter(|e| !e.is_empty() && *e != "octet-stream")
        .unwrap_or("jpg")
        .replace("jpeg", "jpg");
    let suffix = if index == 0 {
        String::new()
    } else {
        format!("_{}", index + 1)
    };

    std::fs::create_dir_all(out_dir)?;
    let target = out_dir.join(format!("{stem}_cover{suffix}.{ext}"));
    std::fs::write(&target, &cover.data)?;
    Ok(target)
}

/// 由图片文件构造封面（自动识别 MIME）。
pub fn cover_from_file(image: &Path) -> Result<CoverArt> {
    if !image.is_file() {
        return Err(TfError::Input(format!("图片不存在：{}", image.display())));
    }
    let data = std::fs::read(image)?;
    if data.len() < 8 {
        return Err(TfError::Input("图片数据过小".into()));
    }
    Ok(CoverArt::new(CoverArt::guess_mime(image), data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_minimal_wav;

    /// 最小 PNG（1x1 透明），仅用于测试字节流。
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn embed_read_export_and_remove_cover() {
        let dir = tempfile::tempdir().unwrap();
        let audio = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);
        assert!(read_first_cover(&audio).unwrap().is_none());

        let cover = CoverArt::new("image/png", PNG.to_vec());
        set_cover(&audio, Some(&cover)).unwrap();
        let read = read_first_cover(&audio).unwrap().unwrap();
        assert_eq!(read.mime, "image/png");
        assert_eq!(read.data, PNG);

        let exported = export_cover(&audio, 0, &dir.path().join("covers")).unwrap();
        assert!(exported.exists());
        assert_eq!(std::fs::read(&exported).unwrap(), PNG);
        assert!(exported.to_string_lossy().ends_with(".png"));

        remove_covers(&audio).unwrap();
        assert!(read_first_cover(&audio).unwrap().is_none());
    }

    #[test]
    fn replacing_cover_keeps_single_entry() {
        let dir = tempfile::tempdir().unwrap();
        let audio = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);
        set_cover(&audio, Some(&CoverArt::new("image/png", PNG.to_vec()))).unwrap();
        set_cover(&audio, Some(&CoverArt::new("image/jpeg", vec![0xFF, 0xD8, 0xFF, 0xE0]))).unwrap();
        let covers = read_covers(&audio).unwrap();
        assert_eq!(covers.len(), 1);
        assert_eq!(covers[0].mime, "image/jpeg");
    }

    #[test]
    fn exporting_missing_cover_errors() {
        let dir = tempfile::tempdir().unwrap();
        let audio = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);
        let err = export_cover(&audio, 0, dir.path()).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }

    #[test]
    fn cover_from_file_guesses_mime() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("art.png");
        std::fs::write(&img, PNG).unwrap();
        let cover = cover_from_file(&img).unwrap();
        assert_eq!(cover.mime, "image/png");
        assert_eq!(cover.len(), PNG.len());
        assert!(cover_from_file(&dir.path().join("nope.jpg")).is_err());
    }

    #[test]
    fn empty_cover_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let audio = write_minimal_wav(&dir.path().join("song.wav"), 8000, 1, 800);
        let err = set_cover(&audio, Some(&CoverArt::new("image/png", Vec::new()))).unwrap_err();
        assert_eq!(err.category(), tf_core::ErrorCategory::Input);
    }
}
