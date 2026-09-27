//! TPDF 抖动与量化（设计方案 §7.6：降位深使用 TPDF 抖动，升位深不加抖动）。

use crate::model::AudioBuffer;
use crate::util::Rng;

/// 量化目标处的满量程（有符号整数格式：16-bit → 32768）。
#[inline]
pub fn full_scale(bits: u16) -> f64 {
    (1u64 << (bits.saturating_sub(1)).min(62)) as f64
}

/// 是否需要抖动：仅当目标位深低于源位深。
pub fn needs_dither(source_bits: Option<u16>, target_bits: u16) -> bool {
    matches!(source_bits, Some(src) if target_bits < src)
}

/// 用 TPDF 抖动就地量化到 `bits` 位（只应在降位深时调用）。
///
/// 返回量化后的最大绝对误差（线性，理论值 ≤ 2 LSB）。
pub fn quantize_in_place(buf: &mut AudioBuffer, bits: u16, seed: u64) -> f64 {
    let bits = bits.clamp(2, 32);
    let scale = full_scale(bits);
    let inv = 1.0 / scale;
    let limit = 1.0 - inv; // 有符号整数格式的正向最大值（避免 +1.0 溢出）
    let mut rng = Rng::new(seed);
    let mut max_err = 0.0f64;
    for v in buf.data.iter_mut() {
        let x = v.clamp(-1.0, limit);
        let dither = rng.next_tpdf(); // (-1, 1) LSB
        let q = ((x * scale + dither).round()).clamp(-scale, scale - 1.0) * inv;
        let err = (q - x).abs();
        if err > max_err {
            max_err = err;
        }
        *v = q;
    }
    max_err
}

/// 不做抖动的硬量化（升位深或无损路径用）。
pub fn quantize_plain_in_place(buf: &mut AudioBuffer, bits: u16) -> f64 {
    let bits = bits.clamp(2, 32);
    let scale = full_scale(bits);
    let inv = 1.0 / scale;
    let limit = 1.0 - inv;
    let mut max_err = 0.0f64;
    for v in buf.data.iter_mut() {
        let x = v.clamp(-1.0, limit);
        let q = (x * scale).round() * inv;
        let err = (q - x).abs();
        if err > max_err {
            max_err = err;
        }
        *v = q;
    }
    max_err
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::sine_mono;

    fn sine_buffer(bits_amplitude: f64) -> AudioBuffer {
        let mono = sine_mono(48_000, 48_000, 997.0, bits_amplitude);
        let mut buf = AudioBuffer::new(48_000, 1, mono.len());
        buf.channel_mut(0).copy_from_slice(&mono);
        buf
    }

    #[test]
    fn dither_error_stays_within_two_lsb() {
        let mut buf = sine_buffer(0.7);
        let max_err = quantize_in_place(&mut buf, 16, 1234);
        let lsb = 1.0 / full_scale(16);
        assert!(max_err <= 2.0 * lsb, "max_err={max_err} lsb={lsb}");
        // 所有样本都应落在 16-bit 网格上
        for v in buf.data.iter() {
            let scaled = v * full_scale(16);
            assert!((scaled - scaled.round()).abs() < 1e-6);
        }
    }

    #[test]
    fn dither_is_deterministic_for_same_seed() {
        let mut a = sine_buffer(0.7);
        let mut b = sine_buffer(0.7);
        quantize_in_place(&mut a, 16, 99);
        quantize_in_place(&mut b, 16, 99);
        assert_eq!(a, b);
        let mut c = sine_buffer(0.7);
        quantize_in_place(&mut c, 16, 100);
        assert_ne!(a, c);
    }

    #[test]
    fn plain_quantize_is_idempotent_after_first_pass() {
        let mut buf = sine_buffer(0.42);
        quantize_plain_in_place(&mut buf, 24);
        let snapshot = buf.clone();
        let err = quantize_plain_in_place(&mut buf, 24);
        assert_eq!(snapshot, buf);
        assert_eq!(err, 0.0);
    }

    #[test]
    fn dither_decision_matches_bit_depth_rule() {
        assert!(needs_dither(Some(32), 24));
        assert!(!needs_dither(Some(16), 24));
        assert!(!needs_dither(None, 16));
    }

    #[test]
    fn quantization_does_not_clip() {
        let mut buf = sine_buffer(1.4); // 超范围信号
        quantize_plain_in_place(&mut buf, 16);
        assert!(buf.data.iter().all(|v| v.abs() <= 1.0));
    }
}
