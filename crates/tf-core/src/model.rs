//! 媒体、标签、音频缓冲的领域模型（设计方案 §4.1 `model.rs`）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TfError};

/// 支持的音频格式。
///
/// * `Ape` / `Dsf` / `Dff` 只可作输入（FFmpeg 无 APE 编码器；不做 PCM→DSD，见 D6/D 限制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AudioFormat {
    Flac,
    Wav,
    Aiff,
    Alac,
    Mp3,
    Aac,
    Ogg,
    Opus,
    Ape,
    WavPack,
    Dsf,
    Dff,
}

impl AudioFormat {
    /// 全部已知格式。
    pub const ALL: [AudioFormat; 12] = [
        AudioFormat::Flac,
        AudioFormat::Wav,
        AudioFormat::Aiff,
        AudioFormat::Alac,
        AudioFormat::Mp3,
        AudioFormat::Aac,
        AudioFormat::Ogg,
        AudioFormat::Opus,
        AudioFormat::Ape,
        AudioFormat::WavPack,
        AudioFormat::Dsf,
        AudioFormat::Dff,
    ];

    /// 稳定字符串 id（用于前后端通信与配置文件）。
    pub fn id(self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff",
            AudioFormat::Alac => "alac",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Aac => "aac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::Ape => "ape",
            AudioFormat::WavPack => "wavpack",
            AudioFormat::Dsf => "dsf",
            AudioFormat::Dff => "dff",
        }
    }

    /// 解析 [`AudioFormat::id`]。
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.trim().to_ascii_lowercase();
        AudioFormat::ALL.into_iter().find(|f| f.id() == id)
    }

    /// UI 展示名。
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Flac => "FLAC",
            AudioFormat::Wav => "WAV",
            AudioFormat::Aiff => "AIFF",
            AudioFormat::Alac => "ALAC (M4A)",
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Aac => "AAC (M4A)",
            AudioFormat::Ogg => "OGG Vorbis",
            AudioFormat::Opus => "Opus",
            AudioFormat::Ape => "APE",
            AudioFormat::WavPack => "WavPack",
            AudioFormat::Dsf => "DSD (DSF)",
            AudioFormat::Dff => "DSD (DFF)",
        }
    }

    /// 输出文件扩展名（不含点）。
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff",
            AudioFormat::Alac | AudioFormat::Aac => "m4a",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::Ape => "ape",
            AudioFormat::WavPack => "wv",
            AudioFormat::Dsf => "dsf",
            AudioFormat::Dff => "dff",
        }
    }

    /// 由文件扩展名推断格式。
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        Some(match ext.as_str() {
            "flac" => AudioFormat::Flac,
            "wav" | "wave" => AudioFormat::Wav,
            "aif" | "aiff" | "aifc" => AudioFormat::Aiff,
            "m4a" | "mp4" | "alac" => AudioFormat::Alac,
            "mp3" | "mp2" => AudioFormat::Mp3,
            "aac" => AudioFormat::Aac,
            "ogg" | "oga" => AudioFormat::Ogg,
            "opus" => AudioFormat::Opus,
            "ape" => AudioFormat::Ape,
            "wv" => AudioFormat::WavPack,
            "dsf" => AudioFormat::Dsf,
            "dff" => AudioFormat::Dff,
            _ => return None,
        })
    }

    /// 由 ffprobe 的 `format_name` / `codec_name` 推断格式。
    ///
    /// 优先使用 codec（更精确），再退回容器名。
    pub fn from_probe(format_name: &str, codec_name: &str) -> Option<Self> {
        let codec = codec_name.to_ascii_lowercase();
        let container = format_name.to_ascii_lowercase();
        match codec.as_str() {
            "flac" => return Some(AudioFormat::Flac),
            "mp3" | "mp3float" => return Some(AudioFormat::Mp3),
            "alac" => return Some(AudioFormat::Alac),
            "aac" | "aac_latm" => return Some(AudioFormat::Aac),
            "vorbis" => return Some(AudioFormat::Ogg),
            "opus" => return Some(AudioFormat::Opus),
            "ape" | "monkeysaudio" => return Some(AudioFormat::Ape),
            "wavpack" => return Some(AudioFormat::WavPack),
            "dsd_lsbf" | "dsd_lsbf_planar" => return Some(AudioFormat::Dsf),
            "dsd_msbf" | "dsd_msbf_planar" => return Some(AudioFormat::Dff),
            _ => {}
        }
        // PCM 家族按容器区分
        if codec.starts_with("pcm_") {
            if container.contains("aiff") || container.contains("aif") {
                return Some(AudioFormat::Aiff);
            }
            if container.contains("wav") || container.contains("wave") {
                return Some(AudioFormat::Wav);
            }
        }
        for token in container.split(',') {
            match token.trim() {
                "flac" => return Some(AudioFormat::Flac),
                "wav" | "wave" => return Some(AudioFormat::Wav),
                "aiff" | "aif" => return Some(AudioFormat::Aiff),
                "mp3" => return Some(AudioFormat::Mp3),
                "ogg" => return Some(AudioFormat::Ogg),
                "opus" => return Some(AudioFormat::Opus),
                "ape" => return Some(AudioFormat::Ape),
                "wv" => return Some(AudioFormat::WavPack),
                "dsf" => return Some(AudioFormat::Dsf),
                "dff" => return Some(AudioFormat::Dff),
                "mov" | "mp4" | "m4a" => return Some(AudioFormat::Alac),
                _ => {}
            }
        }
        None
    }

    /// 该格式能否作为输出（FFmpeg 编码器可用性由 `tf-media` 的 capability 检查再兜底）。
    pub fn can_encode(self) -> bool {
        !matches!(
            self,
            AudioFormat::Ape | AudioFormat::Dsf | AudioFormat::Dff | AudioFormat::WavPack
        )
    }

    /// 是否无损。
    pub fn is_lossless(self) -> bool {
        matches!(
            self,
            AudioFormat::Flac
                | AudioFormat::Wav
                | AudioFormat::Aiff
                | AudioFormat::Alac
                | AudioFormat::Ape
                | AudioFormat::WavPack
                | AudioFormat::Dsf
                | AudioFormat::Dff
        )
    }

    /// 是否 DSD（只做解码，见 D6）。
    pub fn is_dsd(self) -> bool {
        matches!(self, AudioFormat::Dsf | AudioFormat::Dff)
    }

    /// 可作为输出的格式列表（转换页下拉用）。
    pub fn output_formats() -> Vec<AudioFormat> {
        AudioFormat::ALL
            .into_iter()
            .filter(|f| f.can_encode())
            .collect()
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl Serialize for AudioFormat {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for AudioFormat {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        AudioFormat::from_id(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("未知音频格式 id: {raw}")))
    }
}

/// 标准标签字段（设计方案 §8 / R4）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
}

impl Tags {
    /// 该标签集合是否为空（所有字段无值）。
    pub fn is_empty(&self) -> bool {
        self == &Tags::default()
    }

    /// 把空字符串视为“无值”，便于模板渲染。
    pub fn normalized(mut self) -> Self {
        fn clean(v: &mut Option<String>) {
            if let Some(s) = v {
                let t = s.trim();
                if t.is_empty() {
                    *v = None;
                } else if t.len() != s.len() {
                    *s = t.to_string();
                }
            }
        }
        clean(&mut self.title);
        clean(&mut self.artist);
        clean(&mut self.album);
        clean(&mut self.album_artist);
        clean(&mut self.year);
        clean(&mut self.genre);
        clean(&mut self.comment);
        self
    }
}

/// 可批量编辑的标签字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Track,
    Disc,
    Year,
    Genre,
    Comment,
}

impl TagField {
    /// 全部字段。
    pub const ALL: [TagField; 9] = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::AlbumArtist,
        TagField::Track,
        TagField::Disc,
        TagField::Year,
        TagField::Genre,
        TagField::Comment,
    ];

    /// 中文展示名。
    pub fn label(self) -> &'static str {
        match self {
            TagField::Title => "标题",
            TagField::Artist => "艺术家",
            TagField::Album => "专辑",
            TagField::AlbumArtist => "专辑艺术家",
            TagField::Track => "音轨号",
            TagField::Disc => "碟号",
            TagField::Year => "年份",
            TagField::Genre => "流派",
            TagField::Comment => "备注",
        }
    }

    /// 对应模板变量名（`{artist}` 等）。
    pub fn variable(self) -> &'static str {
        match self {
            TagField::Title => "title",
            TagField::Artist => "artist",
            TagField::Album => "album",
            TagField::AlbumArtist => "albumartist",
            TagField::Track => "track",
            TagField::Disc => "disc",
            TagField::Year => "year",
            TagField::Genre => "genre",
            TagField::Comment => "comment",
        }
    }

    /// 读取字段的字符串值。
    pub fn get(self, tags: &Tags) -> Option<String> {
        match self {
            TagField::Title => tags.title.clone(),
            TagField::Artist => tags.artist.clone(),
            TagField::Album => tags.album.clone(),
            TagField::AlbumArtist => tags.album_artist.clone(),
            TagField::Track => tags.track.map(|v| v.to_string()),
            TagField::Disc => tags.disc.map(|v| v.to_string()),
            TagField::Year => tags.year.clone(),
            TagField::Genre => tags.genre.clone(),
            TagField::Comment => tags.comment.clone(),
        }
    }

    /// 写入字段值（空字符串等于清除）。数字字段解析失败返回错误。
    pub fn set(self, tags: &mut Tags, value: Option<String>) -> Result<()> {
        let value = value.and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
        match self {
            TagField::Title => tags.title = value,
            TagField::Artist => tags.artist = value,
            TagField::Album => tags.album = value,
            TagField::AlbumArtist => tags.album_artist = value,
            TagField::Year => tags.year = value,
            TagField::Genre => tags.genre = value,
            TagField::Comment => tags.comment = value,
            TagField::Track | TagField::Disc => {
                let parsed = match &value {
                    None => None,
                    Some(v) => Some(v.parse::<u32>().map_err(|_| {
                        TfError::Input(format!("{} 必须是数字，收到 “{v}”", self.label()))
                    })?),
                };
                if self == TagField::Track {
                    tags.track = parsed;
                } else {
                    tags.disc = parsed;
                }
            }
        }
        Ok(())
    }
}

/// 封面图片（未编码的原始字节）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverArt {
    /// MIME 类型，例如 `image/jpeg`。
    pub mime: String,
    /// 原始图片数据。
    pub data: Vec<u8>,
}

impl CoverArt {
    /// 新建封面。
    pub fn new(mime: impl Into<String>, data: Vec<u8>) -> Self {
        CoverArt {
            mime: mime.into(),
            data,
        }
    }

    /// 由文件扩展名猜测 MIME。
    pub fn guess_mime(path: &std::path::Path) -> String {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("png") => "image/png".into(),
            Some("gif") => "image/gif".into(),
            Some("bmp") => "image/bmp".into(),
            Some("webp") => "image/webp".into(),
            _ => "image/jpeg".into(),
        }
    }

    /// 字节大小。
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 是否没有数据。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// 音源评级（用于列表提示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRating {
    /// 无损高解析（> 48 kHz 或位深 > 16）。
    HiRes,
    /// 普通无损。
    Lossless,
    /// 有损。
    Lossy,
    /// DSD 音源（只可解码）。
    Dsd,
}

impl SourceRating {
    /// 由媒体信息与格式推断。
    pub fn from_media(media: &MediaInfo) -> Option<Self> {
        let format = media.format?;
        if format.is_dsd() {
            return Some(SourceRating::Dsd);
        }
        if !format.is_lossless() {
            return Some(SourceRating::Lossy);
        }
        let hires =
            media.sample_rate.unwrap_or(0) > 48_000 || media.bits_per_sample.unwrap_or(0) > 16;
        Some(if hires {
            SourceRating::HiRes
        } else {
            SourceRating::Lossless
        })
    }

    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            SourceRating::HiRes => "高解析无损",
            SourceRating::Lossless => "无损",
            SourceRating::Lossy => "有损",
            SourceRating::Dsd => "DSD",
        }
    }
}

/// ffprobe（+ 文件系统）得到的媒体信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// 文件路径。
    pub path: PathBuf,
    /// 推断出的音频格式（无法识别则为 `None`）。
    pub format: Option<AudioFormat>,
    /// ffprobe 的 `format_name`。
    pub container: String,
    /// ffprobe 的 `codec_name`。
    pub codec: String,
    /// 采样率（Hz）。
    pub sample_rate: Option<u32>,
    /// 位深（bit）。
    pub bits_per_sample: Option<u16>,
    /// 浮点采样（如 f32 flac）。
    pub is_float: bool,
    /// 声道数。
    pub channels: Option<u16>,
    /// 声道布局，如 `5.1`。
    pub channel_layout: Option<String>,
    /// 时长（秒）。
    pub duration_secs: Option<f64>,
    /// 帧（采样点）数。
    pub frames: Option<u64>,
    /// 码率（bit/s）。
    pub bit_rate: Option<u64>,
    /// 文件大小（字节）。
    pub size_bytes: u64,
}

impl MediaInfo {
    /// 时长的分钟:秒展示。
    pub fn duration_label(&self) -> String {
        match self.duration_secs {
            Some(secs) if secs.is_finite() && secs >= 0.0 => {
                let total = secs.round() as u64;
                format!("{}:{:02}", total / 60, total % 60)
            }
            _ => "-".into(),
        }
    }
}

/// 扫描得到的媒体条目（列表行 + 编辑状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    /// 稳定 id（前端列表 key）。
    pub id: String,
    /// 源文件路径。
    pub path: PathBuf,
    /// 文件名。
    pub file_name: String,
    /// 探测到的媒体信息；探测失败时为 `None`。
    pub media: Option<MediaInfo>,
    /// 标签。
    pub tags: Tags,
    /// 是否含封面。
    pub has_cover: bool,
    /// 探测/读取失败信息。
    pub error: Option<String>,
    /// 已测量的集成响度（LUFS）。
    pub loudness_lufs: Option<f64>,
    /// 已测量的真峰值（dBTP）。
    pub true_peak_dbtp: Option<f64>,
    /// 是否为“假多声道”（8 声道但只有前 2 声道有声）。
    pub fake_multichannel: bool,
    /// 音源评级。
    pub rating: Option<SourceRating>,
}

/// 播放器/处理管线使用的平面 `f64` 音频缓冲。
///
/// 内部布局为 planar：`data[ch * frames + i]`。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// 采样率。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 每声道采样点数。
    pub frames: usize,
    /// 平面数据。
    pub data: Vec<f64>,
}

impl AudioBuffer {
    /// 新建全零缓冲。
    pub fn new(sample_rate: u32, channels: usize, frames: usize) -> Self {
        AudioBuffer {
            sample_rate,
            channels,
            frames,
            data: vec![0.0; channels * frames],
        }
    }

    /// 从交织的 `f32` PCM 构造（FFmpeg `f32le` 输出）。
    pub fn from_interleaved_f32(
        sample_rate: u32,
        channels: usize,
        interleaved: &[f32],
    ) -> Result<Self> {
        if channels == 0 {
            return Err(TfError::Input("声道数不能为 0".into()));
        }
        let frames = interleaved.len() / channels;
        let mut buf = AudioBuffer::new(sample_rate, channels, frames);
        for i in 0..frames {
            for ch in 0..channels {
                buf.data[ch * frames + i] = interleaved[i * channels + ch] as f64;
            }
        }
        Ok(buf)
    }

    /// 从平面 `f64` 数据构造（长度必须是 `channels * frames`）。
    pub fn from_planar(sample_rate: u32, channels: usize, data: Vec<f64>) -> Result<Self> {
        if channels == 0 {
            return Err(TfError::Input("声道数不能为 0".into()));
        }
        if data.len() % channels != 0 {
            return Err(TfError::Internal(format!(
                "平面缓冲长度 {} 不是声道数 {} 的整数倍",
                data.len(),
                channels
            )));
        }
        let frames = data.len() / channels;
        Ok(AudioBuffer {
            sample_rate,
            channels,
            frames,
            data,
        })
    }

    /// 单声道切片（只读）。
    pub fn channel(&self, ch: usize) -> &[f64] {
        let start = ch * self.frames;
        &self.data[start..start + self.frames]
    }

    /// 单声道切片（可变）。
    pub fn channel_mut(&mut self, ch: usize) -> &mut [f64] {
        let start = ch * self.frames;
        &mut self.data[start..start + self.frames]
    }

    /// 时长（秒）。
    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.frames == 0
    }

    /// 样本峰值（线性）。
    pub fn sample_peak(&self) -> f64 {
        self.data.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    }

    /// 整体 RMS（线性，跨声道求均方）。
    pub fn rms(&self) -> f64 {
        if self.data.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.data.iter().map(|v| v * v).sum();
        (sum / self.data.len() as f64).sqrt()
    }

    /// 每声道 RMS。
    pub fn channel_rms(&self) -> Vec<f64> {
        (0..self.channels)
            .map(|ch| {
                let slice = self.channel(ch);
                if slice.is_empty() {
                    0.0
                } else {
                    (slice.iter().map(|v| v * v).sum::<f64>() / slice.len() as f64).sqrt()
                }
            })
            .collect()
    }

    /// 只保留前 `n` 个声道。
    pub fn keep_channels(&self, n: usize) -> Result<AudioBuffer> {
        if n == 0 || n > self.channels {
            return Err(TfError::Input(format!(
                "无法把 {} 声道缩减为 {n} 声道",
                self.channels
            )));
        }
        let mut buf = AudioBuffer::new(self.sample_rate, n, self.frames);
        for ch in 0..n {
            let (_, dst) = buf.data.split_at_mut(ch * self.frames);
            dst[..self.frames].copy_from_slice(self.channel(ch));
        }
        Ok(buf)
    }

    /// 对整段音频施加线性增益（就地）。
    pub fn apply_gain(&mut self, gain: f64) {
        for v in self.data.iter_mut() {
            *v *= gain;
        }
    }

    /// 逐样本增益乘以一条增益曲线（长度必须等于 `frames`）。
    pub fn apply_gain_curve(&mut self, gain: f64, curve: &[f64]) -> Result<()> {
        if curve.len() != self.frames {
            return Err(TfError::Internal(format!(
                "增益曲线长度 {} 与帧数 {} 不一致",
                curve.len(),
                self.frames
            )));
        }
        for ch in 0..self.channels {
            let slice = self.channel_mut(ch);
            for (i, v) in slice.iter_mut().enumerate() {
                *v *= gain * curve[i];
            }
        }
        Ok(())
    }

    /// 交织为 `f32`（发送给 FFmpeg 的标准输入）。
    pub fn to_interleaved_f32(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; self.channels * self.frames];
        for i in 0..self.frames {
            for ch in 0..self.channels {
                out[i * self.channels + ch] = self.data[ch * self.frames + i] as f32;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ids_and_extensions_roundtrip() {
        for f in AudioFormat::ALL {
            assert_eq!(AudioFormat::from_id(f.id()), Some(f));
        }
        // `m4a` 同时用于 ALAC 与 AAC，扩展名只对无歧义格式做双向校验
        for f in AudioFormat::ALL {
            if f == AudioFormat::Aac {
                assert_eq!(
                    AudioFormat::from_extension(f.extension()),
                    Some(AudioFormat::Alac)
                );
                continue;
            }
            assert_eq!(AudioFormat::from_extension(f.extension()), Some(f));
        }
        assert_eq!(
            AudioFormat::from_extension(".FLAC"),
            Some(AudioFormat::Flac)
        );
        assert_eq!(AudioFormat::from_id("bogus"), None);
    }

    #[test]
    fn probe_mapping_covers_common_cases() {
        assert_eq!(
            AudioFormat::from_probe("flac", "flac"),
            Some(AudioFormat::Flac)
        );
        assert_eq!(
            AudioFormat::from_probe("mov,mp4,m4a", "alac"),
            Some(AudioFormat::Alac)
        );
        assert_eq!(
            AudioFormat::from_probe("mov,mp4,m4a", "aac"),
            Some(AudioFormat::Aac)
        );
        assert_eq!(
            AudioFormat::from_probe("wav", "pcm_s24le"),
            Some(AudioFormat::Wav)
        );
        assert_eq!(
            AudioFormat::from_probe("aiff", "pcm_s16be"),
            Some(AudioFormat::Aiff)
        );
        assert_eq!(
            AudioFormat::from_probe("dsf", "dsd_lsbf"),
            Some(AudioFormat::Dsf)
        );
        assert_eq!(AudioFormat::from_probe("", "monkey"), None);
    }

    #[test]
    fn ape_and_dsd_cannot_encode() {
        assert!(!AudioFormat::Ape.can_encode());
        assert!(!AudioFormat::Dsf.can_encode());
        assert!(!AudioFormat::Dff.can_encode());
        assert!(AudioFormat::Flac.can_encode());
        assert!(!AudioFormat::output_formats().contains(&AudioFormat::Ape));
    }

    #[test]
    fn audio_format_serializes_as_id_string() {
        let json = serde_json::to_string(&AudioFormat::Alac).unwrap();
        assert_eq!(json, "\"alac\"");
        let back: AudioFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AudioFormat::Alac);
        assert!(serde_json::from_str::<AudioFormat>("\"bogus\"").is_err());
    }

    #[test]
    fn interleaved_roundtrip_is_exact() {
        let interleaved: Vec<f32> = vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3];
        let buf = AudioBuffer::from_interleaved_f32(48_000, 2, &interleaved).unwrap();
        assert_eq!(buf.frames, 3);
        assert_eq!(
            buf.channel(0),
            &[0.1f32 as f64, 0.2f32 as f64, 0.3f32 as f64]
        );
        assert_eq!(
            buf.channel(1),
            &[-0.1f32 as f64, -0.2f32 as f64, -0.3f32 as f64]
        );
        assert_eq!(buf.to_interleaved_f32(), interleaved);
    }

    #[test]
    fn keep_channels_and_rms() {
        let mut buf = AudioBuffer::new(48_000, 8, 100);
        buf.channel_mut(0).iter_mut().for_each(|v| *v = 0.5);
        buf.channel_mut(1).iter_mut().for_each(|v| *v = 0.5);
        let stereo = buf.keep_channels(2).unwrap();
        assert_eq!(stereo.channels, 2);
        assert!((stereo.rms() - 0.5).abs() < 1e-12);
        assert!(buf.keep_channels(9).is_err());
        let rms = buf.channel_rms();
        assert!((rms[0] - 0.5).abs() < 1e-12);
        assert_eq!(rms[7], 0.0);
    }

    #[test]
    fn tag_field_get_set_roundtrip() {
        let mut tags = Tags::default();
        TagField::Title
            .set(&mut tags, Some(" 歌名 ".into()))
            .unwrap();
        TagField::Track.set(&mut tags, Some("7".into())).unwrap();
        assert_eq!(tags.title.as_deref(), Some("歌名"));
        assert_eq!(tags.track, Some(7));
        assert_eq!(TagField::Track.get(&tags).as_deref(), Some("7"));
        assert!(TagField::Track.set(&mut tags, Some("七".into())).is_err());
        TagField::Track.set(&mut tags, Some("".into())).unwrap();
        assert_eq!(tags.track, None);
    }

    /// 「清除其他标签」的载荷：除「艺术家 / 标题」外全部置空（null），且不报校验错误。
    #[test]
    fn clearing_other_fields_payload_keeps_artist_and_title() {
        let mut tags = Tags::default();
        for field in TagField::ALL {
            let value = match field {
                TagField::Track => "7",
                TagField::Disc => "1",
                _ => "v",
            };
            field.set(&mut tags, Some(value.to_string())).unwrap();
        }
        assert_eq!(tags.track, Some(7));

        // 前端「清除其他标签」按钮实际发送的编辑项
        for field in TagField::ALL {
            if !matches!(field, TagField::Artist | TagField::Title) {
                field.set(&mut tags, None).unwrap();
            }
        }

        let tags = tags.normalized();
        assert_eq!(tags.title.as_deref(), Some("v"));
        assert_eq!(tags.artist.as_deref(), Some("v"));
        assert_eq!(tags.album, None);
        assert_eq!(tags.album_artist, None);
        assert_eq!(tags.track, None);
        assert_eq!(tags.disc, None);
        assert_eq!(tags.year, None);
        assert_eq!(tags.genre, None);
        assert_eq!(tags.comment, None);
    }

    #[test]
    fn rating_detection() {
        let mut media = MediaInfo {
            path: PathBuf::from("x.flac"),
            format: Some(AudioFormat::Flac),
            container: "flac".into(),
            codec: "flac".into(),
            sample_rate: Some(96_000),
            bits_per_sample: Some(24),
            is_float: false,
            channels: Some(2),
            channel_layout: Some("stereo".into()),
            duration_secs: Some(1.0),
            frames: Some(48_000),
            bit_rate: None,
            size_bytes: 0,
        };
        assert_eq!(SourceRating::from_media(&media), Some(SourceRating::HiRes));
        media.sample_rate = Some(44_100);
        media.bits_per_sample = Some(16);
        assert_eq!(
            SourceRating::from_media(&media),
            Some(SourceRating::Lossless)
        );
        media.format = Some(AudioFormat::Ape);
        assert_eq!(
            SourceRating::from_media(&media),
            Some(SourceRating::Lossless)
        );
        media.format = Some(AudioFormat::Mp3);
        assert_eq!(SourceRating::from_media(&media), Some(SourceRating::Lossy));
        media.format = Some(AudioFormat::Dsf);
        assert_eq!(SourceRating::from_media(&media), Some(SourceRating::Dsd));
        assert_eq!(media.duration_label(), "0:01");
    }
}
