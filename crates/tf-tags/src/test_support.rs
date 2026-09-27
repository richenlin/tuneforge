//! 测试辅助：手写最小合法 WAV 文件（不依赖 ffmpeg）。

use std::path::{Path, PathBuf};

/// 生成最小 16-bit PCM WAV 的字节内容。
pub fn minimal_wav_bytes(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = frames as u32 * block_align as u32;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    // 一段低电平正弦，方便需要真实样本的测试复用
    for i in 0..frames {
        let t = i as f64 / sample_rate as f64;
        let v = (0.1 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 32767.0) as i16;
        for _ in 0..channels {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// 写一个最小 WAV 文件并返回路径。
pub fn write_minimal_wav(path: &Path, sample_rate: u32, channels: u16, frames: usize) -> PathBuf {
    let bytes = minimal_wav_bytes(sample_rate, channels, frames);
    std::fs::write(path, bytes).expect("写入测试 WAV 失败");
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_well_formed() {
        let bytes = minimal_wav_bytes(48_000, 2, 100);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        assert_eq!(riff_size as usize, bytes.len() - 8);
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(data_len as usize, 100 * 2 * 2);
        assert_eq!(bytes.len(), 44 + data_len as usize);
    }

    #[test]
    fn file_writer_creates_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_minimal_wav(&dir.path().join("t.wav"), 44_100, 1, 441);
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 44 + 441 * 2);
    }
}
