//! 真峰值（True Peak）测量：4 倍过采样 polyphase FIR（设计方案 §7.3 / §7.4）。
//!
//! * 原型滤波器为 Kaiser 窗 sinc，默认 β≈8.6、每相 12 抽头（BS.1770-4 Annex 2 的规格），
//!   相位内做直流归一化，保证常数信号（DC）增益为 1。
//! * 使用**零相位**（居中）卷积：第 `i` 个输出对应输入样本 `i` 附近的过采样峰值，
//!   因此包络与输入在时间上对齐 —— 限幅器直接消费该包络即可。
//! * 边界用边缘延拓（clamp）处理，避免首尾出现虚假的过冲/欠冲。

use serde::{Deserialize, Serialize};

use crate::model::AudioBuffer;
use crate::parallel::map_ordered;

/// 默认过采样倍数。
pub const DEFAULT_FACTOR: usize = 4;
/// 默认每相抽头数。
pub const DEFAULT_TAPS: usize = 12;
/// 默认 Kaiser β（≈ BS.1770-4 规格）。
pub const DEFAULT_BETA: f64 = 8.6;

/// 修正贝塞尔函数 `I0`（Kaiser 窗用）。
fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let mut k = 1.0f64;
    while k <= 200.0 {
        term *= (half / k) * (half / k);
        sum += term;
        if term < 1e-18 * sum {
            break;
        }
        k += 1.0;
    }
    sum
}

/// 多相插值滤波器。
#[derive(Debug, Clone, PartialEq)]
pub struct InterpFilter {
    factor: usize,
    taps: usize,
    /// 长度 `factor * taps`，索引 `phase + k * factor`。
    coeffs: Vec<f64>,
}

impl InterpFilter {
    /// 构造 Kaiser 窗 sinc 插值滤波器。
    pub fn kaiser(factor: usize, taps: usize, beta: f64) -> Self {
        let factor = factor.max(2);
        let taps = taps.max(2);
        let len = factor * taps;
        let center = (len as f64 - 1.0) / 2.0;
        let mut coeffs = vec![0.0f64; len];
        let i0_beta = bessel_i0(beta);
        for (n, c) in coeffs.iter_mut().enumerate() {
            let t = (n as f64 - center) / factor as f64;
            let sinc = if t.abs() < 1e-12 {
                1.0
            } else {
                let pt = std::f64::consts::PI * t;
                pt.sin() / pt
            };
            let r = (2.0 * n as f64 - (len as f64 - 1.0)) / (len as f64 - 1.0);
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0_beta;
            *c = sinc * w;
        }
        let mut filter = InterpFilter {
            factor,
            taps,
            coeffs,
        };
        filter.normalize_phases();
        filter
    }

    /// 默认参数（4x，12 抽头，β=8.6）。
    pub fn default_filter() -> Self {
        Self::kaiser(DEFAULT_FACTOR, DEFAULT_TAPS, DEFAULT_BETA)
    }

    /// 每个相位单独做直流归一化（保证 DC 增益 1、相位间无电平差）。
    fn normalize_phases(&mut self) {
        for p in 0..self.factor {
            let sum: f64 = (0..self.taps)
                .map(|k| self.coeffs[p + k * self.factor])
                .sum();
            if sum.abs() > 1e-12 {
                for k in 0..self.taps {
                    self.coeffs[p + k * self.factor] /= sum;
                }
            }
        }
    }

    /// 过采样倍数。
    pub fn factor(&self) -> usize {
        self.factor
    }

    /// 每相抽头数。
    pub fn taps(&self) -> usize {
        self.taps
    }

    /// 计算逐样本真峰值包络（长度与 `x` 相同，零相位对齐）。
    pub fn envelope(&self, x: &[f64]) -> Vec<f64> {
        let n = x.len();
        if n == 0 {
            return Vec::new();
        }
        let d = (self.taps / 2) as isize;
        let mut out = vec![0.0f64; n];
        for (i, slot) in out.iter_mut().enumerate() {
            let mut peak = 0.0f64;
            for p in 0..self.factor {
                let mut acc = 0.0f64;
                for k in 0..self.taps {
                    let idx = i as isize + d - k as isize;
                    let sample = if idx < 0 {
                        x[0]
                    } else if idx as usize >= n {
                        x[n - 1]
                    } else {
                        x[idx as usize]
                    };
                    acc += self.coeffs[p + k * self.factor] * sample;
                }
                let a = acc.abs();
                if a > peak {
                    peak = a;
                }
            }
            *slot = peak;
        }
        out
    }

    /// 该信号的真峰值（线性）。
    pub fn peak(&self, x: &[f64]) -> f64 {
        self.envelope(x).into_iter().fold(0.0f64, f64::max)
    }

    /// 该信号真峰值的 dBTP 表示。
    pub fn peak_dbtp(&self, x: &[f64]) -> f64 {
        linear_to_dbtp(self.peak(x))
    }
}

/// 线性幅度 → dBTP。
#[inline]
pub fn linear_to_dbtp(x: f64) -> f64 {
    if x <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * x.log10()
    }
}

/// dBTP → 线性幅度。
#[inline]
pub fn dbtp_to_linear(dbtp: f64) -> f64 {
    10f64.powf(dbtp / 20.0)
}

/// 整段缓冲的真峰值（线性），按声道并行。
pub fn true_peak_with(filter: &InterpFilter, buf: &AudioBuffer) -> f64 {
    if buf.frames == 0 || buf.channels == 0 {
        return 0.0;
    }
    let per_channel = map_ordered(buf.channels, 24_000, |ch| filter.peak(buf.channel(ch)));
    per_channel.into_iter().fold(0.0f64, f64::max)
}

/// 用默认滤波器测真峰值（线性）。
pub fn true_peak(buf: &AudioBuffer) -> f64 {
    true_peak_with(&InterpFilter::default_filter(), buf)
}

/// 整段缓冲的真峰值（dBTP）。
pub fn true_peak_dbtp(buf: &AudioBuffer) -> f64 {
    linear_to_dbtp(true_peak(buf))
}

/// 提高位深/编码前的安全上限：`-1 dBTP` 对应的线性幅度。
pub fn default_ceiling_linear() -> f64 {
    dbtp_to_linear(-1.0)
}

/// 真峰值测量结果（用于列表展示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruePeakReport {
    /// 真峰值（dBTP）。
    pub true_peak_dbtp: f64,
    /// 样本峰值（dBFS）。
    pub sample_peak_dbfs: f64,
    /// 过采样峰值与样本峰值之比（dB），越大说明越多 inter-sample peak。
    pub oversampling_gain_db: f64,
}

/// 测量并给出对比信息。
pub fn analyze(buf: &AudioBuffer) -> TruePeakReport {
    let tp = true_peak(buf);
    let sp = buf.sample_peak();
    TruePeakReport {
        true_peak_dbtp: linear_to_dbtp(tp),
        sample_peak_dbfs: linear_to_dbtp(sp),
        oversampling_gain_db: linear_to_dbtp(tp) - linear_to_dbtp(sp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::sine_mono;

    #[test]
    fn dc_signal_has_unity_gain() {
        let f = InterpFilter::default_filter();
        let dc = vec![0.5f64; 1000];
        let peak = f.peak(&dc);
        assert!((peak - 0.5).abs() < 1e-9, "DC 增益应精确为 1，实际 {peak}");
        let env = f.envelope(&dc);
        assert_eq!(env.len(), dc.len());
        assert!(env.iter().all(|v| (v - 0.5).abs() < 1e-9));
    }

    #[test]
    fn low_frequency_sine_is_measured_accurately() {
        let f = InterpFilter::default_filter();
        let s = sine_mono(48_000, 48_000, 997.0, 0.5);
        let tp = f.peak(&s);
        let err_db = linear_to_dbtp(tp) - linear_to_dbtp(0.5);
        assert!(err_db.abs() < 0.1, "997 Hz 误差 {err_db:.4} dB");
    }

    /// 12 kHz 正弦在 48 kHz 采样下样本峰值只有 0.707A，真峰值应接近 A。
    #[test]
    fn intersample_peak_is_detected() {
        let f = InterpFilter::default_filter();
        let amp = 0.8;
        let frames = 4800;
        let s: Vec<f64> = (0..frames)
            .map(|n| {
                let t = n as f64 / 48_000.0;
                amp * (2.0 * std::f64::consts::PI * 12_000.0 * t + std::f64::consts::FRAC_PI_4)
                    .sin()
            })
            .collect();
        let sample_peak = s.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let tp = f.peak(&s);
        assert!(
            (sample_peak - amp * std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "样本峰值 {sample_peak}"
        );
        let ratio = tp / sample_peak;
        assert!(ratio > 1.3 && ratio < 1.45, "过采样/样本峰值 = {ratio}");
        let err_db = linear_to_dbtp(tp) - linear_to_dbtp(amp);
        assert!(err_db.abs() < 0.5, "真峰值误差 {err_db:.3} dB");
    }

    #[test]
    fn true_peak_is_never_below_sample_peak() {
        let mut rng = crate::util::Rng::new(7);
        let frames = 20_000;
        let mut buf = AudioBuffer::new(48_000, 2, frames);
        for ch in 0..2 {
            let slice = buf.channel_mut(ch);
            for v in slice.iter_mut() {
                *v = (rng.next_f64() * 2.0 - 1.0) * 0.7;
            }
        }
        let tp = true_peak(&buf);
        assert!(tp >= buf.sample_peak() - 1e-12);
    }

    #[test]
    fn envelope_length_and_alignment_are_stable() {
        let f = InterpFilter::default_filter();
        let impulse: Vec<f64> = (0..200).map(|i| if i == 100 { 1.0 } else { 0.0 }).collect();
        let env = f.envelope(&impulse);
        assert_eq!(env.len(), 200);
        // 冲激响应的峰值应出现在冲激附近（零相位对齐）
        let (argmax, _) = env
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!((argmax as isize - 100).abs() <= 3, "峰值位置 {argmax}");
    }

    #[test]
    fn dbtp_conversions() {
        assert!((dbtp_to_linear(-1.0) - 0.891_250_9).abs() < 1e-6);
        assert!((linear_to_dbtp(1.0)).abs() < 1e-12);
        assert!(linear_to_dbtp(0.0).is_infinite());
        assert!((default_ceiling_linear() - 0.891_250_9).abs() < 1e-6);
    }

    #[test]
    fn analyze_reports_oversampling_gain() {
        let frames = 4800;
        let s: Vec<f64> = (0..frames)
            .map(|n| {
                let t = n as f64 / 48_000.0;
                0.8 * (2.0 * std::f64::consts::PI * 12_000.0 * t + std::f64::consts::FRAC_PI_4)
                    .sin()
            })
            .collect();
        let mut buf = AudioBuffer::new(48_000, 1, frames);
        buf.channel_mut(0).copy_from_slice(&s);
        let report = analyze(&buf);
        assert!(report.oversampling_gain_db > 2.0);
        assert!(report.true_peak_dbtp > report.sample_peak_dbfs);
    }
}
