//! 小型数值工具：dB 换算与确定性随机数（抖动用）。

/// 线性幅度转 dBFS；0 返回 `f64::NEG_INFINITY`。
#[inline]
pub fn linear_to_db(x: f64) -> f64 {
    if x == 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * x.abs().log10()
    }
}

/// dB 转线性幅度。
#[inline]
pub fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// 平方（功率）转 dB。
#[inline]
pub fn power_to_db(x: f64) -> f64 {
    10.0 * x.max(f64::MIN_POSITIVE).log10()
}

/// 把值夹到 `[lo, hi]`。
#[inline]
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// 确定性 xorshift64* 随机数发生器。
///
/// 抖动需要可复现：给定种子，任何机器上的结果都一致（便于回归测试）。
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// 用种子创建；0 会被替换为一个非零常量。
    pub fn new(seed: u64) -> Self {
        Rng {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// 下一个 64 位随机数。
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `[0, 1)` 上的均匀分布。
    pub fn next_f64(&mut self) -> f64 {
        // 取高 53 位，得到 [0,1)
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `(-1, 1)` 上的三角分布（两个均匀分布之差，用于 TPDF 抖动）。
    pub fn next_tpdf(&mut self) -> f64 {
        self.next_f64() - self.next_f64()
    }
}

/// 生成测试用正弦信号（单声道），便于 DSP 单测复用。
#[cfg(any(test, feature = "test-signals"))]
pub fn sine_mono(sample_rate: u32, frames: usize, freq: f64, amplitude: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(frames);
    for n in 0..frames {
        let t = n as f64 / sample_rate as f64;
        out.push(amplitude * (2.0 * std::f64::consts::PI * freq * t).sin());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip() {
        assert!((db_to_linear(linear_to_db(0.5)) - 0.5).abs() < 1e-12);
        assert!((linear_to_db(0.891_250_94) + 1.0).abs() < 1e-3);
        assert!(linear_to_db(0.0).is_infinite());
    }

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.next_f64();
            assert_eq!(x, b.next_f64());
            assert!((0.0..1.0).contains(&x));
        }
        let mut c = Rng::new(7);
        assert!((c.next_tpdf()).abs() < 1.0);
    }

    #[test]
    fn sine_has_expected_peak() {
        let s = sine_mono(48_000, 4800, 997.0, 0.5);
        let peak = s.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 1e-3, "peak={peak}");
    }
}
