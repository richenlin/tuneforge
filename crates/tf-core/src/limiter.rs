//! 音量归一化：增益 + 真峰值限幅（复刻设计方案 §7.4 的已验证算法）。
//!
//! 算法步骤：
//! 1. （可选）假多声道折混 —— 响度必须在折混后的信号上测量。
//! 2. 测量集成响度 `L`，基础增益 `g = 10^((T - L) / 20)`。
//! 3. 计算 `x·g` 的真峰值包络 `tp`；若 `max(tp) ≤ C` 直接线性增益输出。
//! 4. 否则做 look-ahead 限幅：`req = min(1, C / tp)`，向前取 3 ms 最小值，
//!    dB 域 1 ms attack / 40 ms release 平滑，`curve = min(smoothed, req_la)`。
//! 5. 输出 `y = x · g · curve`。
//! 6. **最终安全 trim**：测量 `y` 的真峰值 `tp_out`，若超天花板则整体线性衰减
//!    `C / tp_out` 并重新编码（线性增益对真峰值是线性缩放，一次即收敛）。
//!
//! 第 6 步是关键：时变增益会破坏“增益在重建滤波器窗口内近似恒定”的假设，
//! 单靠逐点包络无法保证真峰值。

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::downmix;
use crate::error::{Result, TfError};
use crate::loudness;
use crate::model::AudioBuffer;
use crate::parallel::map_ordered;
use crate::truepeak::{self, InterpFilter};
use crate::util::{db_to_linear, linear_to_db};

/// 归一化参数（默认值 = 设计方案 D8）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeParams {
    /// 目标集成响度（LUFS）。
    pub target_lufs: f64,
    /// 真峰值天花板（dBTP）；`None` 表示不做真峰值限幅。
    pub ceiling_dbtp: Option<f64>,
    /// 前视时间（毫秒）。
    pub lookahead_ms: f64,
    /// 攻击时间（毫秒）。
    pub attack_ms: f64,
    /// 释放时间（毫秒）。
    pub release_ms: f64,
    /// 是否把假多声道折混为立体声。
    pub downmix_fake_multichannel: bool,
}

impl Default for NormalizeParams {
    fn default() -> Self {
        NormalizeParams {
            target_lufs: -14.0,
            ceiling_dbtp: Some(-1.0),
            lookahead_ms: 3.0,
            attack_ms: 1.0,
            release_ms: 40.0,
            downmix_fake_multichannel: true,
        }
    }
}

impl NormalizeParams {
    /// 目标响度预设。
    pub fn preset(target_lufs: f64) -> Self {
        NormalizeParams {
            target_lufs,
            ..Default::default()
        }
    }

    /// 天花板（线性）。
    pub fn ceiling_linear(&self) -> Option<f64> {
        self.ceiling_dbtp.map(truepeak::dbtp_to_linear)
    }
}

/// 归一化结果与统计。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NormalizeOutcome {
    /// 处理后的音频。
    #[serde(skip)]
    pub buffer: AudioBuffer,
    /// 处理前测量的集成响度（LUFS）。
    pub measured_lufs: f64,
    /// 基础增益（dB）。
    pub base_gain_db: f64,
    /// 是否触发了限幅。
    pub limited: bool,
    /// 限幅引入的最大衰减（dB，正数表示衰减量）。
    pub limiter_reduction_db: f64,
    /// 最终安全 trim（dB，正数表示衰减量）。
    pub final_trim_db: f64,
    /// 安全 trim 迭代次数。
    pub trim_iterations: usize,
    /// 输出真峰值（dBTP）。
    pub output_true_peak_dbtp: f64,
    /// 输出样本峰值（dBFS）。
    pub output_sample_peak_dbfs: f64,
    /// 输出实测集成响度（LUFS）。
    pub output_lufs: Option<f64>,
    /// 实际总增益（dB，含限幅与 trim）。
    pub effective_gain_db: f64,
    /// 是否做了假多声道折混。
    pub downmixed: bool,
}

impl NormalizeOutcome {
    /// 与目标响度的偏差（LU）。
    pub fn delta_to_target(&self, target: f64) -> Option<f64> {
        self.output_lufs.map(|l| l - target)
    }
}

/// 执行归一化（不修改入参）。
pub fn normalize(input: &AudioBuffer, params: &NormalizeParams) -> Result<NormalizeOutcome> {
    if input.frames == 0 || input.channels == 0 {
        return Err(TfError::Input("音频为空，无法归一化".into()));
    }

    // 1. 折混（响度必须在折混后的信号上测量，§7.3）
    let mut was_downmixed = false;
    let downmixed = if params.downmix_fake_multichannel && downmix::is_fake_multichannel(input) {
        was_downmixed = true;
        downmix::downmix_fake_multichannel(input)?
    } else {
        input.clone()
    };

    // 2. 测量
    let measured = loudness::measure(&downmixed);
    let measured_lufs = measured.integrated_lufs.ok_or_else(|| {
        TfError::Input("音频过短或近乎静音，无法测量集成响度（至少需要 400 ms 有效音频）".into())
    })?;

    let filter = InterpFilter::default_filter();

    // 3. 基础增益
    let g = db_to_linear(params.target_lufs - measured_lufs);
    let ceiling = params.ceiling_linear();

    let mut out = downmixed.clone();
    out.apply_gain(g);

    let mut limited = false;
    let mut limiter_reduction_db = 0.0;

    if let Some(c) = ceiling {
        // 逐声道真峰值包络（线性缩放性质：envelope(x·g) = envelope(x)·g）
        let envelopes: Vec<Vec<f64>> =
            map_ordered(out.channels, 24_000, |ch| filter.envelope(out.channel(ch)));

        let mut req = vec![1.0f64; out.frames];
        let mut needs_limiting = false;
        for ch in 0..out.channels {
            let env = &envelopes[ch];
            for i in 0..out.frames {
                let tp = env[i];
                if tp > c {
                    needs_limiting = true;
                    let r = c / tp;
                    if r < req[i] {
                        req[i] = r;
                    }
                }
            }
        }

        if needs_limiting {
            limited = true;
            let look_frames =
                ((params.lookahead_ms / 1000.0) * out.sample_rate as f64).round() as usize;
            let req_la = min_filter_forward(&req, look_frames);
            let curve = smooth_gain_curve(
                &req_la,
                out.sample_rate,
                params.attack_ms,
                params.release_ms,
            );
            limiter_reduction_db = curve
                .iter()
                .map(|v| -linear_to_db(*v))
                .fold(0.0f64, f64::max);
            for ch in 0..out.channels {
                let slice = out.channel_mut(ch);
                for (i, v) in slice.iter_mut().enumerate() {
                    *v *= curve[i];
                }
            }
        }
    }

    // 6. 最终安全 trim（最多 3 次迭代，通常 1 次收敛）
    let mut final_trim_db = 0.0;
    let mut trim_iterations = 0;
    if let Some(c) = ceiling {
        for _ in 0..3 {
            trim_iterations += 1;
            let tp = truepeak::true_peak_with(&filter, &out);
            if tp <= c {
                break;
            }
            let trim = c / tp;
            out.apply_gain(trim);
            final_trim_db += -linear_to_db(trim);
        }
    }

    let output_true_peak_dbtp = truepeak::true_peak_dbtp(&out);
    let output_lufs = loudness::integrated_lufs(&out);
    let effective_gain_db = match output_lufs {
        Some(l) => l - measured_lufs,
        None => linear_to_db(g) - final_trim_db,
    };

    Ok(NormalizeOutcome {
        measured_lufs,
        base_gain_db: linear_to_db(g),
        limited,
        limiter_reduction_db: limiter_reduction_db.max(0.0),
        final_trim_db,
        trim_iterations,
        output_true_peak_dbtp,
        output_sample_peak_dbfs: linear_to_db(out.sample_peak()),
        output_lufs,
        effective_gain_db,
        downmixed: was_downmixed,
        buffer: out,
    })
}

/// 对该缓冲施加固定线性增益，并用真峰值天花板做安全衰减（转换页“无损增益”用）。
pub fn apply_gain_with_ceiling(
    input: &AudioBuffer,
    gain_db: f64,
    ceiling_dbtp: Option<f64>,
) -> (AudioBuffer, f64) {
    let mut out = input.clone();
    out.apply_gain(db_to_linear(gain_db));
    let mut trim_db = 0.0;
    if let Some(c) = ceiling_dbtp.map(truepeak::dbtp_to_linear) {
        let tp = truepeak::true_peak(&out);
        if tp > c && tp > 0.0 {
            let trim = c / tp;
            out.apply_gain(trim);
            trim_db = -linear_to_db(trim);
        }
    }
    (out, trim_db)
}

/// 向前滑动最小值：`out[i] = min(x[i..i+window])`（window=0 时返回原值）。
///
/// 单调队列实现，复杂度 O(n)。
fn min_filter_forward(x: &[f64], window: usize) -> Vec<f64> {
    let n = x.len();
    if n == 0 {
        return Vec::new();
    }
    if window == 0 {
        return x.to_vec();
    }
    let mut out = vec![0.0f64; n];
    let mut deque: VecDeque<usize> = VecDeque::new();
    // 从右往左扫描，维护窗口内最小值索引
    for i in (0..n).rev() {
        while let Some(&back) = deque.back() {
            if x[back] >= x[i] {
                deque.pop_back();
            } else {
                break;
            }
        }
        deque.push_back(i);
        let deadline = i + window - 1;
        while let Some(&front) = deque.front() {
            if front > deadline {
                deque.pop_front();
            } else {
                break;
            }
        }
        out[i] = x[*deque.front().unwrap_or(&i)];
    }
    out
}

/// dB 域双速平滑（attack / release），并保证结果不超过需求曲线。
fn smooth_gain_curve(req: &[f64], sample_rate: u32, attack_ms: f64, release_ms: f64) -> Vec<f64> {
    let n = req.len();
    if n == 0 {
        return Vec::new();
    }
    let fs = sample_rate.max(1) as f64;
    let attack_coef = time_constant_coef(attack_ms, fs);
    let release_coef = time_constant_coef(release_ms, fs);

    let to_db = |v: f64| -> f64 {
        if v <= 0.0 {
            -240.0
        } else {
            (20.0 * v.log10()).max(-240.0)
        }
    };

    let mut out = vec![0.0f64; n];
    let mut state = to_db(req[0]);
    for i in 0..n {
        let target = to_db(req[i]);
        let coef = if target < state {
            attack_coef
        } else {
            release_coef
        };
        state += (target - state) * coef;
        // 保证曲线不超过需求（curve ≤ req）
        let db = state.min(target);
        out[i] = 10f64.powf(db / 20.0);
    }
    out
}

fn time_constant_coef(time_ms: f64, sample_rate: f64) -> f64 {
    if time_ms <= 0.0 {
        return 1.0;
    }
    let tau = time_ms / 1000.0 * sample_rate;
    (1.0 - (-1.0 / tau).exp()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::sine_mono;

    fn stereo(buf: AudioBuffer) -> AudioBuffer {
        buf
    }

    fn stereo_sine(sample_rate: u32, frames: usize, freq: f64, amp: f64) -> AudioBuffer {
        let mono = sine_mono(sample_rate, frames, freq, amp);
        let mut buf = AudioBuffer::new(sample_rate, 2, frames);
        buf.channel_mut(0).copy_from_slice(&mono);
        buf.channel_mut(1).copy_from_slice(&mono);
        buf
    }

    #[test]
    fn linear_gain_hits_target_when_no_limiting_needed() {
        let buf = stereo_sine(48_000, 48_000 * 3, 1000.0, 0.1);
        let out = normalize(&buf, &NormalizeParams::default()).unwrap();
        assert!(!out.limited);
        assert_eq!(out.trim_iterations, 1);
        assert!(
            (out.output_lufs.unwrap() + 14.0).abs() < 0.2,
            "{:?}",
            out.output_lufs
        );
        assert!(out.output_true_peak_dbtp <= -1.0 + 1e-6);
        // 立体声 -20 dBFS 正弦 ≈ -20 LUFS → 目标 -14 LUFS 需要约 +6 dB
        assert!((out.base_gain_db - 6.0).abs() < 1.0, "{}", out.base_gain_db);
    }

    /// 12 kHz 正弦有很强的 inter-sample peak；把目标定在 +6 LU 一定会触发限幅。
    fn hot_intersample_signal(frames: usize) -> AudioBuffer {
        let amp = 0.9;
        let mut buf = AudioBuffer::new(48_000, 2, frames);
        for ch in 0..2 {
            let slice = buf.channel_mut(ch);
            for (n, v) in slice.iter_mut().enumerate() {
                let t = n as f64 / 48_000.0;
                *v = amp
                    * (2.0 * std::f64::consts::PI * 12_000.0 * t + std::f64::consts::FRAC_PI_4)
                        .sin();
            }
        }
        buf
    }

    #[test]
    fn limiting_engages_and_respects_ceiling() {
        let frames = 48_000 * 3;
        let buf = hot_intersample_signal(frames);
        let measured = crate::loudness::integrated_lufs(&buf).unwrap();
        let params = NormalizeParams {
            target_lufs: measured + 6.0,
            ..NormalizeParams::default()
        };
        let out = normalize(&buf, &params).unwrap();
        assert!(out.limited, "应触发限幅: {out:?}");
        assert!(out.limiter_reduction_db > 0.0);
        assert!(
            out.output_true_peak_dbtp <= -1.0 + 0.01,
            "真峰值 {} 超过天花板",
            out.output_true_peak_dbtp
        );
        // 限幅后不应超出目标响度
        assert!(out.output_lufs.unwrap() <= params.target_lufs + 0.1);
        // 基础增益应为 +6 LU
        assert!((out.base_gain_db - 6.0).abs() < 0.2, "{}", out.base_gain_db);
        assert!(out.buffer.data.iter().all(|v| v.is_finite()));
        assert!(out.buffer.data.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn disabled_ceiling_means_no_limiting() {
        let frames = 48_000;
        let buf = hot_intersample_signal(frames);
        let measured = crate::loudness::integrated_lufs(&buf).unwrap();
        let params = NormalizeParams {
            target_lufs: measured + 6.0,
            ceiling_dbtp: None,
            ..NormalizeParams::default()
        };
        let out = normalize(&buf, &params).unwrap();
        assert!(!out.limited);
        assert_eq!(out.final_trim_db, 0.0);
        assert!(out.output_true_peak_dbtp > -1.0);
    }

    #[test]
    fn quiet_signal_is_boosted_without_limiting() {
        let buf = stereo_sine(48_000, 48_000 * 2, 1000.0, 0.001);
        let params = NormalizeParams::preset(-23.0);
        let out = normalize(&buf, &params).unwrap();
        assert!(!out.limited);
        assert!((out.output_lufs.unwrap() + 23.0).abs() < 0.2);
    }

    #[test]
    fn stereo_image_is_preserved() {
        let frames = 48_000 * 2;
        let mut buf = AudioBuffer::new(48_000, 2, frames);
        let left = sine_mono(48_000, frames, 1000.0, 0.8);
        let right = sine_mono(48_000, frames, 3000.0, 0.2);
        buf.channel_mut(0).copy_from_slice(&left);
        buf.channel_mut(1).copy_from_slice(&right);
        // 目标低于原始响度 → 纯线性衰减，左右比例必须严格保持
        let measured = crate::loudness::integrated_lufs(&buf).unwrap();
        let params = NormalizeParams {
            target_lufs: measured - 6.0,
            ..NormalizeParams::default()
        };
        let input_ratio = 0.8 / 0.2;
        let out = normalize(&buf, &params).unwrap();
        let out_ratio = peak(&out.buffer.channel(0)) / peak(&out.buffer.channel(1));
        assert!(
            (out_ratio - input_ratio).abs() < 0.05,
            "左右声道比例应保持：{input_ratio} → {out_ratio}"
        );
    }

    fn peak(x: &[f64]) -> f64 {
        x.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    }

    #[test]
    fn normalize_is_deterministic() {
        let buf = stereo_sine(48_000, 48_000 * 2, 997.0, 0.6);
        let measured = crate::loudness::integrated_lufs(&buf).unwrap();
        let params = NormalizeParams {
            target_lufs: measured + 4.0,
            ..NormalizeParams::default()
        };
        let a = normalize(&buf, &params).unwrap();
        let b = normalize(&buf, &params).unwrap();
        assert_eq!(a.buffer.data, b.buffer.data);
        assert_eq!(a, b);
    }

    #[test]
    fn silent_input_is_rejected() {
        let buf = AudioBuffer::new(48_000, 2, 48_000 * 2);
        let err = normalize(&buf, &NormalizeParams::default()).unwrap_err();
        assert_eq!(err.category(), crate::error::ErrorCategory::Input);
    }

    #[test]
    fn fake_multichannel_is_folded_before_measuring() {
        let frames = 48_000 * 2;
        let mut buf = AudioBuffer::new(48_000, 8, frames);
        let mono = sine_mono(48_000, frames, 1000.0, 0.2);
        buf.channel_mut(0).copy_from_slice(&mono);
        buf.channel_mut(1).copy_from_slice(&mono);
        let out = normalize(&buf, &NormalizeParams::default()).unwrap();
        assert!(out.downmixed);
        assert_eq!(out.buffer.channels, 2);
        assert!((out.output_lufs.unwrap() + 14.0).abs() < 0.2);
    }

    #[test]
    fn min_filter_forward_matches_naive_implementation() {
        let x = vec![3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        let naive: Vec<f64> = (0..x.len())
            .map(|i| {
                let end = (i + 3).min(x.len());
                x[i..end].iter().fold(f64::INFINITY, |m, v| m.min(*v))
            })
            .collect();
        assert_eq!(min_filter_forward(&x, 3), naive);
        assert_eq!(min_filter_forward(&x, 0), x);
        assert!(min_filter_forward(&[], 3).is_empty());
    }

    #[test]
    fn gain_curve_never_exceeds_requirement() {
        let sample_rate = 48_000u32;
        let mut req = vec![1.0f64; 48_000];
        // 中间一段需要衰减，两侧恢复
        for (i, v) in req.iter_mut().enumerate() {
            if (10_000..30_000).contains(&i) {
                *v = 0.25 + 0.25 * ((i - 10_000) as f64 / 20_000.0);
            }
        }
        let curve = smooth_gain_curve(&req, sample_rate, 1.0, 40.0);
        for (c, r) in curve.iter().zip(req.iter()) {
            assert!(*c <= *r + 1e-12, "curve {c} > req {r}");
        }
    }

    #[test]
    fn apply_gain_with_ceiling_trims_peaks() {
        let frames = 4800;
        let mut buf = AudioBuffer::new(48_000, 1, frames);
        let slice = buf.channel_mut(0);
        for (n, v) in slice.iter_mut().enumerate() {
            let t = n as f64 / 48_000.0;
            *v = 0.9
                * (2.0 * std::f64::consts::PI * 12_000.0 * t + std::f64::consts::FRAC_PI_4).sin();
        }
        let (out, trim) = apply_gain_with_ceiling(&buf, 6.0, Some(-1.0));
        assert!(trim > 0.0);
        assert!(truepeak::true_peak(&out) <= truepeak::dbtp_to_linear(-1.0) + 1e-9);
        let (out2, trim2) = apply_gain_with_ceiling(&buf, -6.0, Some(-1.0));
        assert_eq!(trim2, 0.0);
        assert!(out2.sample_peak() < buf.sample_peak());
    }

    #[test]
    fn stereo_helper_identity() {
        let buf = stereo_sine(48_000, 4800, 997.0, 0.1);
        assert_eq!(stereo(buf.clone()), buf);
    }
}
