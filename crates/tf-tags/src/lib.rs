//! 标签与封面读写（设计方案 §4.1 / §8），基于 `lofty`。
//!
//! 职责边界：只做标签/封面，不碰音频样本。标签写入失败仅返回错误，
//! 不会破坏已经写好的音频（流水线里音频先落盘，标签后写）。
#![forbid(unsafe_code)]

pub mod cover;
pub mod read;
pub mod write;

#[cfg(test)]
pub mod test_support;

pub use cover::{
    cover_from_file, export_cover, read_covers, read_first_cover, remove_covers, set_cover,
    set_cover_with,
};
pub use read::{
    apply_tags_to_lofty, has_existing_tag, primary_tag_type, read_snapshot, read_tags,
    read_tags_and_cover, tags_from_lofty, TagSnapshot,
};
pub use write::{copy_tags, update_field, write_tags, CopyPolicy, CopyReport, WriteOptions};
