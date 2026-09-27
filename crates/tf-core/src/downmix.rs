//! 多声道处理（设计方案 §7.5）：假多声道检测与折混、常规立体声/单声道下混。

use serde::{Deserialize, Serialize};

use crate::error::{Result, TfError};
use crate::model::AudioBuffer;
use crate::util::linear_to_db;

/// 认定为“假多声道”所需的最小声道数。
pub const FAKE_MULTICHANNEL_MIN_CHANNELS: usize = 6;
/// 默认静音阈值（dBFS）。
pub const DEFAULT_SILENCE_THRESHOLD_DBFS: f64 = -90.0;

/// 假多声道检测报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FakeMultichannelReport {
    /// 是否判定为假多声道。
    pub is_fake: bool,
    /// 总声道数。
    pub channels: usize,
    /// 有声声道索引。
    pub active_channels: Vec<usize>,
    /// 静音声道索引。
    pub silent_channels: Vec<usize>,
    /// 最响的“静音”声道（dBFS）。
    pub loudest_silent_dbfs: f64,
    /// 每个声道的 RMS（dBFS）。
    pub channel_rms_dbfs: Vec<f64>,
}

impl FakeMultichannelReport {
    /// 中文提示文案（列表标记用）。
    pub fn note(&self) -> String {
        if self.is_fake {
            format!(
                "假多声道：{} 声道中第 {} 声道静音（最响 {:.1} dBFS）",
                self.channels,
                self.silent_channels
                    .iter()
                    .map(|c| (c + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                self.loudest_silent_dbfs
            )
        } else {
            String::new()
        }
    }
}

/// 检测“假多声道”：声道数 ≥ 6 且除前两个声道外全部静音。
pub fn detect_fake_multichannel(buf: &AudioBuffer, threshold_dbfs: f64) -> FakeMultichannelReport {
    let rms = buf.channel_rms();
    let rms_db: Vec<f64> = rms.iter().map(|v| linear_to_db(*v)).collect();
    let mut active = Vec::new();
    let mut silent = Vec::new();
    for (ch, db) in rms_db.iter().enumerate() {
        if *db > threshold_dbfs {
            active.push(ch);
        } else {
            silent.push(ch);
        }
    }

    let tail_silent = buf.channels >= FAKE_MULTICHANNEL_MIN_CHANNELS
        && (2..buf.channels).all(|ch| rms_db[ch] <= threshold_dbfs);
    let loudest_silent_dbfs = rms_db
        .iter()
        .enumerate()
        .filter(|(ch, _)| *ch >= 2)
        .map(|(_, db)| *db)
        .fold(f64::NEG_INFINITY, f64::max);

    FakeMultichannelReport {
        is_fake: tail_silent,
        channels: buf.channels,
        active_channels: active,
        silent_channels: silent,
        loudest_silent_dbfs,
        channel_rms_dbfs: rms_db,
    }
}

/// 用默认阈值检测。
pub fn is_fake_multichannel(buf: &AudioBuffer) -> bool {
    detect_fake_multichannel(buf, DEFAULT_SILENCE_THRESHOLD_DBFS).is_fake
}

/// 假多声道折混：只取前两个声道（不做普通下混，避免引入相位/增益变化）。
pub fn downmix_fake_multichannel(buf: &AudioBuffer) -> Result<AudioBuffer> {
    if buf.channels < 2 {
        return Err(TfError::Input(format!(
            "假多声道折混要求至少 2 声道，当前 {}",
            buf.channels
        )));
    }
    buf.keep_channels(2)
}

/// 通用下混到立体声（ITU-R BS.775 风格矩阵）。
///
/// 声道顺序按 FFmpeg 的默认顺序：
/// `FL FR FC LFE BL BR FLC FRC`（>= 7 声道时尾部视为环绕）。
///
/// **电平策略**：按标准系数求和（FL/FR = 1.0、FC/环绕 = 0.7071、LFE 丢弃），
/// **不按系数和归一化** —— 后者会把“只有部分声道有内容”的素材整体压低十几 dB
/// （6 声道 ÷3.828 ≈ −11.7 dB，8 声道 ≈ −14.4 dB），听感上就是声场/电平被无辜削弱。
/// 仅当求和后峰值确实超过 0 dBFS 时，才用一个**全片统一的常数**缩放，
/// 既避免削波，又不引入任何动态处理（相对电平与相位关系保持不变）。
pub fn downmix_to_stereo(buf: &AudioBuffer) -> Result<AudioBuffer> {
    if buf.channels <= 2 {
        return Ok(buf.clone());
    }
    let wl = stereo_matrix(buf.channels, true);
    let wr = stereo_matrix(buf.channels, false);

    let mut out = AudioBuffer::new(buf.sample_rate, 2, buf.frames);
    let mut peak = 0.0f64;
    for i in 0..buf.frames {
        let mut l = 0.0;
        let mut r = 0.0;
        for ch in 0..buf.channels {
            let s = buf.data[ch * buf.frames + i];
            l += wl[ch] * s;
            r += wr[ch] * s;
        }
        peak = peak.max(l.abs()).max(r.abs());
        out.data[i] = l;
        out.data[buf.frames + i] = r;
    }
    if peak > 1.0 {
        let k = 1.0 / peak;
        for v in out.data.iter_mut() {
            *v *= k;
        }
    }
    Ok(out)
}

/// 通用下混到单声道（L/R 等权、FC/环绕 0.7071、LFE 丢弃）。
///
/// 与 [`downmix_to_stereo`] 一致：不做与内容无关的固定衰减，只在会削波时整体缩放。
/// 注：反相内容相抵消是折混的数学必然（立体声信息在单声道下不可恢复），不是 bug。
pub fn downmix_to_mono(buf: &AudioBuffer) -> Result<AudioBuffer> {
    if buf.channels == 1 {
        return Ok(buf.clone());
    }
    let weights = mono_matrix(buf.channels);
    let mut out = AudioBuffer::new(buf.sample_rate, 1, buf.frames);
    let mut peak = 0.0f64;
    for i in 0..buf.frames {
        let mut acc = 0.0;
        for ch in 0..buf.channels {
            acc += weights[ch] * buf.data[ch * buf.frames + i];
        }
        peak = peak.max(acc.abs());
        out.data[i] = acc;
    }
    if peak > 1.0 {
        let k = 1.0 / peak;
        for v in out.data.iter_mut() {
            *v *= k;
        }
    }
    Ok(out)
}

/// LFE 在 FFmpeg 默认布局中的下标（5.1 / 6.1 / 7.1 均为第 4 个声道）。
///
/// 3ch(FL FR FC)、4ch(FL FR BL BR quad)、5ch(FL FR FC BL BR) 没有 LFE。
fn lfe_index(channels: usize) -> Option<usize> {
    match channels {
        6 | 7 | 8 => Some(3),
        _ => None,
    }
}

/// 各声道到 (左, 右) 的增益（ITU-R BS.775）。
///
/// 关键点：FL **只**进左、FR **只**进右（不得互串），FC/环绕各 −3 dB 进对应声道，LFE 丢弃。
/// 布局按 FFmpeg 默认顺序：
/// * 3ch `FL FR FC`、4ch `FL FR BL BR`、5ch `FL FR FC BL BR`
/// * 6ch `FL FR FC LFE BL BR`、7ch `FL FR FC LFE BC SL SR`、8ch `FL FR FC LFE BL BR FLC FRC`
/// * 其它声道数：前两个当 L/R，其余 –3 dB 进双声道，并按 [`lfe_index`] 判断是否丢 LFE。
fn stereo_gains(channels: usize) -> Vec<(f64, f64)> {
    const S: f64 = 0.7071; // −3 dB
    const DROP: (f64, f64) = (0.0, 0.0);
    match channels {
        3 => vec![(1.0, 0.0), (0.0, 1.0), (S, S)],
        4 => vec![(1.0, 0.0), (0.0, 1.0), (S, 0.0), (0.0, S)],
        5 => vec![(1.0, 0.0), (0.0, 1.0), (S, S), (S, 0.0), (0.0, S)],
        6 => vec![(1.0, 0.0), (0.0, 1.0), (S, S), DROP, (S, 0.0), (0.0, S)],
        7 => vec![
            (1.0, 0.0),
            (0.0, 1.0),
            (S, S),
            DROP,
            (S, S),
            (S, 0.0),
            (0.0, S),
        ],
        8 => vec![
            (1.0, 0.0),
            (0.0, 1.0),
            (S, S),
            DROP,
            (S, 0.0),
            (0.0, S),
            (S, S),
            (S, S),
        ],
        n => {
            let mut v = vec![(S, S); n];
            v[0] = (1.0, 0.0);
            v[1] = (0.0, 1.0);
            if let Some(lfe) = lfe_index(n) {
                v[lfe] = DROP;
            }
            v
        }
    }
}

fn stereo_matrix(channels: usize, left: bool) -> Vec<f64> {
    stereo_gains(channels)
        .into_iter()
        .map(|(l, r)| if left { l } else { r })
        .collect()
}

fn mono_matrix(channels: usize) -> Vec<f64> {
    let mut w = vec![0.7071f64; channels];
    if !w.is_empty() {
        w[0] = 1.0;
    }
    if channels > 1 {
        w[1] = 1.0;
    }
    if let Some(lfe) = lfe_index(channels) {
        w[lfe] = 0.0;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eight_channel_with_silent_tail() -> AudioBuffer {
        let frames = 4800;
        let mut buf = AudioBuffer::new(48_000, 8, frames);
        for ch in 0..2 {
            let slice = buf.channel_mut(ch);
            for (i, v) in slice.iter_mut().enumerate() {
                *v = 0.4 * ((i as f64) * 0.01).sin();
            }
        }
        for ch in 2..8 {
            let slice = buf.channel_mut(ch);
            for v in slice.iter_mut() {
                *v = 1e-9; // 实际静音
            }
        }
        buf
    }

    #[test]
    fn fake_multichannel_is_detected_and_folded() {
        let buf = eight_channel_with_silent_tail();
        let report = detect_fake_multichannel(&buf, DEFAULT_SILENCE_THRESHOLD_DBFS);
        assert!(report.is_fake, "{report:?}");
        assert_eq!(report.active_channels, vec![0, 1]);
        assert_eq!(report.silent_channels.len(), 6);
        assert!(report.note().contains("假多声道"));

        let folded = downmix_fake_multichannel(&buf).unwrap();
        assert_eq!(folded.channels, 2);
        assert_eq!(folded.frames, buf.frames);
        assert_eq!(folded.channel(0), buf.channel(0));
        assert_eq!(folded.channel(1), buf.channel(1));
    }

    #[test]
    fn real_multichannel_is_not_flagged() {
        let mut buf = eight_channel_with_silent_tail();
        buf.channel_mut(4).iter_mut().for_each(|v| *v = 0.2);
        let report = detect_fake_multichannel(&buf, DEFAULT_SILENCE_THRESHOLD_DBFS);
        assert!(!report.is_fake);
        assert_eq!(report.note(), "");
    }

    #[test]
    fn stereo_input_is_untouched_by_generic_downmix() {
        let mut buf = AudioBuffer::new(48_000, 2, 100);
        buf.channel_mut(0).iter_mut().for_each(|v| *v = 0.5);
        buf.channel_mut(1).iter_mut().for_each(|v| *v = -0.5);
        assert_eq!(downmix_to_stereo(&buf).unwrap(), buf);
        let mono = downmix_to_mono(&buf).unwrap();
        assert_eq!(mono.channels, 1);
        // L/R 反相 → 单声道为 0
        assert!(mono.data.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn generic_downmix_keeps_peak_bounded() {
        let mut buf = AudioBuffer::new(48_000, 6, 100);
        for ch in 0..6 {
            buf.channel_mut(ch).iter_mut().for_each(|v| *v = 0.9);
        }
        let stereo = downmix_to_stereo(&buf).unwrap();
        assert!(stereo.sample_peak() <= 1.0 + 1e-9);
        let mono = downmix_to_mono(&buf).unwrap();
        assert!(mono.sample_peak() <= 1.0 + 1e-9);
        // 相关性拉满时要“刚好顶到 0 dBFS”，而不是按系数和白白衰减
        assert!(
            (stereo.sample_peak() - 1.0).abs() < 1e-9,
            "{:.4}",
            stereo.sample_peak()
        );
    }

    #[test]
    fn loud_surround_is_scaled_by_one_constant_only() {
        // 5.1 全声道同相 0.9：求和会超 0 dBFS，此时用单一常数缩放（不引入动态处理）
        let mut buf = AudioBuffer::new(48_000, 6, 32);
        for ch in 0..6 {
            buf.channel_mut(ch).iter_mut().for_each(|v| *v = 0.9);
        }
        let out = downmix_to_stereo(&buf).unwrap();
        let peak = out.sample_peak();
        assert!(peak <= 1.0 + 1e-9);
        assert!((peak - 1.0).abs() < 1e-9, "应缩放到刚好不削波：{peak}");
        // 缩放是全片统一的：同一帧内所有样本共享一个比例
        let ratio_l0 = out.channel(0)[0] / buf.channel(0)[0];
        let ratio_r0 = out.channel(1)[0] / buf.channel(1)[0];
        assert!((ratio_l0 - ratio_r0).abs() < 1e-12);
    }

    #[test]
    fn sparse_multichannel_keeps_its_level() {
        // 回归一：旧实现按系数和（6 声道 -> 3.828）归一化，“只有部分声道有内容”的素材
        // 会整体掉 ≈ −11.7 dB；现改为仅在实际削波时用单一常数缩放。
        let mut buf = AudioBuffer::new(48_000, 6, 64);
        buf.channel_mut(0).iter_mut().for_each(|v| *v = 0.5);
        let out = downmix_to_stereo(&buf).unwrap();
        assert_eq!(out.channel(0), buf.channel(0), "FL 应 1:1 进入左声道");
        assert!(
            out.channel(1).iter().all(|v| *v == 0.0),
            "FL 不得串到右声道"
        );

        // 回归二：FR 同理
        let mut buf = AudioBuffer::new(48_000, 6, 64);
        buf.channel_mut(1).iter_mut().for_each(|v| *v = -0.25);
        let out = downmix_to_stereo(&buf).unwrap();
        assert_eq!(out.channel(1), buf.channel(1), "FR 应 1:1 进入右声道");
        assert!(
            out.channel(0).iter().all(|v| *v == 0.0),
            "FR 不得串到左声道"
        );

        // 单声道：只有 L 有内容时也不衰减
        let mut buf = AudioBuffer::new(48_000, 6, 64);
        buf.channel_mut(0).iter_mut().for_each(|v| *v = 0.5);
        let mono = downmix_to_mono(&buf).unwrap();
        assert!((mono.data[0] - 0.5).abs() < 1e-12, "{}", mono.data[0]);
    }

    #[test]
    fn channel_layouts_map_surrounds_to_the_right_side() {
        // 4 声道 quad：BL 只进左、BR 只进右，且不得被当成 LFE 丢弃
        let mut quad = AudioBuffer::new(48_000, 4, 8);
        quad.channel_mut(2).iter_mut().for_each(|v| *v = 1.0);
        quad.channel_mut(3).iter_mut().for_each(|v| *v = 1.0);
        let out = downmix_to_stereo(&quad).unwrap();
        assert!((out.channel(0)[0] - 0.7071).abs() < 1e-12);
        assert!((out.channel(1)[0] - 0.7071).abs() < 1e-12);

        // 5.0（FL FR FC BL BR，无 LFE）：index 3 是左环绕，不能被�当 LFE 丢掉
        let mut five = AudioBuffer::new(48_000, 5, 8);
        five.channel_mut(3).iter_mut().for_each(|v| *v = 1.0);
        let out = downmix_to_stereo(&five).unwrap();
        assert!((out.channel(0)[0] - 0.7071).abs() < 1e-12, "左环绕应保留");
        assert_eq!(out.channel(1)[0], 0.0);

        // 5.1：index 3 是 LFE，必须丢弃
        let mut six = AudioBuffer::new(48_000, 6, 8);
        six.channel_mut(3).iter_mut().for_each(|v| *v = 1.0);
        let out = downmix_to_stereo(&six).unwrap();
        assert_eq!(out.channel(0)[0], 0.0, "LFE 应丢弃");
        assert_eq!(out.channel(1)[0], 0.0);
    }

    #[test]
    fn fake_fold_requires_two_channels() {
        let buf = AudioBuffer::new(48_000, 1, 10);
        assert!(downmix_fake_multichannel(&buf).is_err());
    }
}
