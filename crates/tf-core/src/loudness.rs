//! 集成响度测量：ITU-R BS.1770-4 / EBU R128（设计方案 §7.3）。
//!
//! 实现要点：
//! * K 加权 = 高架滤波器（pre-filter）+ RLB 高通，系数按采样率用双线性变换推导，
//!   公式与 libebur128 完全一致，因此结果可与 ffmpeg `ebur128` 滤镜对齐。
//! * 门限：绝对门限 `-70 LUFS`，相对门限 `(门限后响度 - 10 LU)`。
//! * 块长 400 ms，步长 100 ms（75% 重叠）；同时给出最大瞬时（400 ms）与
//!   最大短时（3 s）响度，便于 UI 展示。
//!
//! 注意：测量必须在**折混之后**的信号上进行（§7.3），否则结果与实际输出不一致。

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::AudioBuffer;

/// BS.1770 响度公式常数。
pub const LOUDNESS_OFFSET: f64 = -0.691;
/// 绝对门限（LUFS）。
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// 相对门限（LU）。
pub const RELATIVE_GATE_LU: f64 = -10.0;
/// 块长（毫秒）。
pub const BLOCK_MS: u32 = 400;
/// 短时窗口（毫秒）。
pub const SHORT_TERM_MS: u32 = 3000;

/// 二阶 IIR（biquad）系数，直接形式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    /// 单位增益直通（单测用）。
    pub fn passthrough() -> Self {
        Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    /// 该滤波器在给定频率处的幅度响应（线性）。
    pub fn magnitude_at(&self, sample_rate: f64, freq: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * freq / sample_rate;
        // H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)
        let (cw1, sw1) = ((-w).cos(), (-w).sin());
        let (cw2, sw2) = ((-2.0 * w).cos(), (-2.0 * w).sin());
        let num_re = self.b0 + self.b1 * cw1 + self.b2 * cw2;
        let num_im = self.b1 * sw1 + self.b2 * sw2;
        let den_re = 1.0 + self.a1 * cw1 + self.a2 * cw2;
        let den_im = self.a1 * sw1 + self.a2 * sw2;
        let num = (num_re * num_re + num_im * num_im).sqrt();
        let den = (den_re * den_re + den_im * den_im).sqrt();
        if den == 0.0 {
            0.0
        } else {
            num / den
        }
    }
}

/// biquad 的转置直接 II 型状态。
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    z1: f64,
    z2: f64,
}

impl BiquadState {
    /// 处理一个样本。
    #[inline]
    pub fn process(&mut self, c: &Biquad, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }

    /// 清零（重新开始测量）。
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// K 加权滤波器的两级。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KWeighting {
    /// 高架滤波器（BS.1770-4 表 1 的泛化推导）。
    pub pre: Biquad,
    /// RLB 高通（BS.1770-4 表 2）。
    pub rlb: Biquad,
}

/// 按采样率推导 K 加权系数。
pub fn k_weighting(sample_rate: u32) -> KWeighting {
    let fs = sample_rate.max(1) as f64;

    // 第一级：高架滤波器
    let f0 = 1681.974_450_955_533;
    let g = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let pre = Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };

    // 第二级：RLB 高通（与 libebur128 的写法保持一致：b 不归一化）
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let rlb = Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };

    KWeighting { pre, rlb }
}

/// 声道加权（BS.1770-4 表 3 / 表 4 的常用布局近似）。
///
/// * 前置声道（L/R/C）权重 1.0
/// * 环绕声道权重 1.41
/// * LFE 不计入（权重 0.0）
///
/// 对于未知布局，采用“前 3 声道 1.0、其余 1.41、第 4 声道视为 LFE（当声道数 >= 6）”。
pub fn channel_weights(channels: usize) -> Vec<f64> {
    match channels {
        0 => Vec::new(),
        1 => vec![1.0],
        2 => vec![1.0, 1.0],
        3 => vec![1.0, 1.0, 1.0],
        4 => vec![1.0, 1.0, 1.41, 1.41],
        5 => vec![1.0, 1.0, 1.0, 0.0, 1.41],
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41],
        7 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41],
        _ => {
            let mut w = vec![1.0; channels];
            for (i, item) in w.iter_mut().enumerate() {
                *item = match i {
                    0 | 1 | 2 => 1.0,
                    3 => 0.0,
                    _ => 1.41,
                };
            }
            w
        }
    }
}

/// 响度测量结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoudnessMeasurement {
    /// 集成响度（LUFS）；信号过短或全静音时为 `None`。
    pub integrated_lufs: Option<f64>,
    /// 最大瞬时响度（400 ms）。
    pub max_momentary_lufs: Option<f64>,
    /// 最大短时响度（3 s）。
    pub max_short_term_lufs: Option<f64>,
    /// 取样峰值（dBFS）。
    pub sample_peak_dbfs: f64,
    /// 总块数。
    pub block_count: usize,
    /// 通过门限的块数。
    pub gated_block_count: usize,
    /// 相对门限值（LUFS）。
    pub relative_gate_lufs: Option<f64>,
}

/// 流式响度计。
#[derive(Debug, Clone)]
pub struct LoudnessMeter {
    sample_rate: u32,
    channels: usize,
    weights: Vec<f64>,
    k: KWeighting,
    states: Vec<(BiquadState, BiquadState)>,
    hop_frames: usize,
    hop_pos: usize,
    hop_sum: f64,
    /// 已完成的 100 ms 跳段（加权均方和），最多保留短时窗口所需数量。
    hops: VecDeque<f64>,
    /// 全部完整的 400 ms 块（加权均方和），用于门限计算。
    blocks: Vec<f64>,
    max_momentary: Option<f64>,
    max_short_term: Option<f64>,
    sample_peak: f64,
}

impl LoudnessMeter {
    /// 新建测量器。
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let hop_frames = ((sample_rate as u64 * 100) / 1000).max(1) as usize;
        LoudnessMeter {
            sample_rate,
            channels,
            weights: channel_weights(channels),
            k: k_weighting(sample_rate),
            states: vec![(BiquadState::default(), BiquadState::default()); channels],
            hop_frames,
            hop_pos: 0,
            hop_sum: 0.0,
            hops: VecDeque::with_capacity(31),
            blocks: Vec::new(),
            max_momentary: None,
            max_short_term: None,
            sample_peak: 0.0,
        }
    }

    /// 采样率。
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 声道数。
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// 送入一段音频（必须与测量器采样率/声道数一致）。
    pub fn push(&mut self, buf: &AudioBuffer) {
        if buf.channels != self.channels || buf.sample_rate != self.sample_rate {
            // 调用方契约：先构造匹配的测量器。为安全起见忽略不匹配的数据。
            return;
        }
        for i in 0..buf.frames {
            let mut block_sum = 0.0;
            for ch in 0..self.channels {
                let x = buf.data[ch * buf.frames + i];
                let (pre_state, rlb_state) = &mut self.states[ch];
                let y = rlb_state.process(&self.k.rlb, pre_state.process(&self.k.pre, x));
                block_sum += self.weights[ch] * y * y;
                let a = x.abs();
                if a > self.sample_peak {
                    self.sample_peak = a;
                }
            }
            self.hop_sum += block_sum;
            self.hop_pos += 1;
            if self.hop_pos == self.hop_frames {
                self.finish_hop();
            }
        }
    }

    fn finish_hop(&mut self) {
        // 跳段内先取**均值**（BS.1770 的块响度是均方值，不是求和）
        let hop_mean = self.hop_sum / self.hop_frames as f64;
        self.hops.push_back(hop_mean);
        while self.hops.len() > 30 {
            self.hops.pop_front();
        }
        self.hop_sum = 0.0;
        self.hop_pos = 0;

        if self.hops.len() >= 4 {
            let z: f64 = self.hops.iter().rev().take(4).sum::<f64>() / 4.0;
            let loudness = loudness_of_power(z);
            if loudness > ABSOLUTE_GATE_LUFS {
                self.blocks.push(z);
                self.max_momentary = Some(match self.max_momentary {
                    Some(m) => m.max(loudness),
                    None => loudness,
                });
            }
        }
        if self.hops.len() >= 30 {
            let z: f64 = self.hops.iter().sum::<f64>() / 30.0;
            let loudness = loudness_of_power(z);
            if loudness > ABSOLUTE_GATE_LUFS {
                self.max_short_term = Some(match self.max_short_term {
                    Some(m) => m.max(loudness),
                    None => loudness,
                });
            }
        }
    }

    /// 当前（尚未结束）的集成响度估计，可用于进度展示。
    pub fn current_lufs(&self) -> Option<f64> {
        self.measure().integrated_lufs
    }

    /// 结束测量并返回结果。
    pub fn measure(&self) -> LoudnessMeasurement {
        let integrated_lufs = {
            // 绝对门限
            let gated: Vec<f64> = self
                .blocks
                .iter()
                .copied()
                .filter(|z| loudness_of_power(*z) > ABSOLUTE_GATE_LUFS)
                .collect();
            let (integrated, relative_gate) = if gated.is_empty() {
                (None, None)
            } else {
                let mean_abs = gated.iter().sum::<f64>() / gated.len() as f64;
                let gate = loudness_of_power(mean_abs) + RELATIVE_GATE_LU;
                // 相对门限
                let kept: Vec<f64> = gated
                    .iter()
                    .copied()
                    .filter(|z| loudness_of_power(*z) >= gate)
                    .collect();
                let mean_rel = if kept.is_empty() {
                    mean_abs
                } else {
                    kept.iter().sum::<f64>() / kept.len() as f64
                };
                (Some(loudness_of_power(mean_rel)), Some(gate))
            };
            (integrated, relative_gate)
        };

        LoudnessMeasurement {
            integrated_lufs: integrated_lufs.0,
            max_momentary_lufs: self.max_momentary,
            max_short_term_lufs: self.max_short_term,
            sample_peak_dbfs: if self.sample_peak == 0.0 {
                f64::NEG_INFINITY
            } else {
                crate::util::linear_to_db(self.sample_peak)
            },
            block_count: self.blocks.len(),
            gated_block_count: integrated_lufs.1.map(|_| self.blocks.len()).unwrap_or(0),
            relative_gate_lufs: integrated_lufs.1,
        }
    }
}

/// 由加权均方和计算块响度（LUFS）。
#[inline]
fn loudness_of_power(z: f64) -> f64 {
    if z <= 0.0 {
        f64::NEG_INFINITY
    } else {
        LOUDNESS_OFFSET + 10.0 * z.log10()
    }
}

/// 便捷函数：一次性测量整个缓冲。
pub fn measure(buf: &AudioBuffer) -> LoudnessMeasurement {
    let mut meter = LoudnessMeter::new(buf.sample_rate, buf.channels.max(1));
    meter.push(buf);
    meter.measure()
}

/// 测量集成响度（LUFS）；无有效块时返回 `None`。
pub fn integrated_lufs(buf: &AudioBuffer) -> Option<f64> {
    measure(buf).integrated_lufs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::sine_mono;

    fn stereo_sine(sample_rate: u32, frames: usize, freq: f64, amplitude: f64) -> AudioBuffer {
        let mono = sine_mono(sample_rate, frames, freq, amplitude);
        let mut buf = AudioBuffer::new(sample_rate, 2, frames);
        buf.channel_mut(0).copy_from_slice(&mono);
        buf.channel_mut(1).copy_from_slice(&mono);
        buf
    }

    /// BS.1770-4 参考：单声道 997 Hz 正弦、峰值幅度 0.1（-20 dBFS）→ -23.0 LUFS。
    #[test]
    fn single_channel_reference_tone_matches_bs1770() {
        let frames = 48_000 * 5;
        let mono = sine_mono(48_000, frames, 997.0, 0.1);
        let mut buf = AudioBuffer::new(48_000, 1, frames);
        buf.channel_mut(0).copy_from_slice(&mono);
        let m = measure(&buf);
        let l = m.integrated_lufs.expect("应有集成响度");
        assert!((l + 23.0).abs() < 0.1, "期望 ≈ -23.0 LUFS，实际 {l:.3}");
    }

    /// EBU Tech 3341 测试 1：立体声 1 kHz 正弦、每声道 -23 dBFS → -23.0 LUFS。
    #[test]
    fn stereo_ebu_reference_tone_matches() {
        let frames = 48_000 * 5;
        let amp = crate::util::db_to_linear(-23.0);
        let buf = stereo_sine(48_000, frames, 1000.0, amp);
        let l = measure(&buf).integrated_lufs.expect("应有集成响度");
        assert!((l + 23.0).abs() < 0.15, "期望 ≈ -23.0 LUFS，实际 {l:.3}");
    }

    /// K 加权在 1 kHz 处约为 +0.7 dB（这正是 -0.691 常数的来历）。
    #[test]
    fn k_weighting_gain_at_1khz_is_about_0_7db() {
        let k = k_weighting(48_000);
        let g_pre = k.pre.magnitude_at(48_000.0, 1000.0);
        let g_rlb = k.rlb.magnitude_at(48_000.0, 1000.0);
        let total_db = crate::util::linear_to_db(g_pre * g_rlb);
        assert!(total_db > 0.4 && total_db < 1.0, "实际 {total_db:.3} dB");
    }

    /// 电平翻倍 = +6.02 LU。
    #[test]
    fn doubling_amplitude_adds_6_lu() {
        let frames = 48_000 * 3;
        let a = stereo_sine(48_000, frames, 1000.0, 0.1);
        let mut b = a.clone();
        b.apply_gain(2.0);
        let la = integrated_lufs(&a).unwrap();
        let lb = integrated_lufs(&b).unwrap();
        assert!((lb - la - 6.0206).abs() < 0.02, "Δ={:.4}", lb - la);
    }

    /// 静音 → 无有效块。
    #[test]
    fn silence_has_no_integrated_loudness() {
        let buf = AudioBuffer::new(48_000, 2, 48_000 * 2);
        let m = measure(&buf);
        assert!(m.integrated_lufs.is_none());
        assert!(m.sample_peak_dbfs.is_infinite());
    }

    /// 时长不足 400 ms 时无法形成块。
    #[test]
    fn too_short_signal_yields_none() {
        let buf = stereo_sine(48_000, 4800, 1000.0, 0.5); // 100 ms
        assert!(integrated_lufs(&buf).is_none());
    }

    /// 多声道权重：LFE 不计入，环绕声道权重大于前置声道。
    #[test]
    fn channel_weights_follow_layout_rules() {
        assert_eq!(channel_weights(1), vec![1.0]);
        assert_eq!(channel_weights(2), vec![1.0, 1.0]);
        let w6 = channel_weights(6);
        assert_eq!(w6[3], 0.0);
        assert!(w6[4] > w6[0]);
        let w8 = channel_weights(8);
        assert_eq!(w8.len(), 8);
        assert_eq!(w8[3], 0.0);
    }

    /// K 加权高架滤波器必须稳定（极点在单位圆内）。
    #[test]
    fn k_weighting_is_stable() {
        for sr in [44_100u32, 48_000, 88_200, 96_000, 176_400, 192_000, 2_822_400] {
            let k = k_weighting(sr);
            for b in [k.pre, k.rlb] {
                // |z| = sqrt(a2) < 1
                assert!(b.a2.abs() < 1.0, "sr={sr} a2={}", b.a2);
                assert!(b.a1.abs() < 3.0, "sr={sr} a1={}", b.a1);
            }
        }
    }

    /// 分块送入与整体送入结果一致（流式实现正确）。
    #[test]
    fn streaming_equals_whole_buffer() {
        let buf = stereo_sine(48_000, 48_000 * 2 + 1234, 997.0, 0.3);
        let whole = measure(&buf).integrated_lufs.unwrap();

        let mut meter = LoudnessMeter::new(48_000, 2);
        let mut offset = 0usize;
        while offset < buf.frames {
            let n = 4096.min(buf.frames - offset);
            let mut chunk = AudioBuffer::new(48_000, 2, n);
            for ch in 0..2 {
                chunk
                    .channel_mut(ch)
                    .copy_from_slice(&buf.channel(ch)[offset..offset + n]);
            }
            meter.push(&chunk);
            offset += n;
        }
        let streamed = meter.measure().integrated_lufs.unwrap();
        assert!((whole - streamed).abs() < 1e-9, "{whole} vs {streamed}");
    }

    /// 短时与瞬时响度应 >= 集成响度（门限只降不升）。
    #[test]
    fn momentary_and_short_term_are_above_integrated() {
        let buf = stereo_sine(48_000, 48_000 * 4, 1000.0, 0.05);
        let m = measure(&buf);
        let integrated = m.integrated_lufs.unwrap();
        assert!(m.max_momentary_lufs.unwrap() >= integrated - 0.01);
        assert!(m.max_short_term_lufs.unwrap() >= integrated - 0.01);
        assert!(m.block_count > 0);
    }
}
