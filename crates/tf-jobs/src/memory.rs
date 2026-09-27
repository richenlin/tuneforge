//! 内存闸门：限制「同时在内存里展开的文件」的总量。
//!
//! DSP 路径（归一化 / 需要降位深抖动 / 折混的转换）会把整首曲子解码成
//! `f64` 平面缓冲，一首 5 分钟立体声 ≈ 210 MB，30 分钟 ≈ 1.2 GB。
//! 线程数一多就可能同时展开好几首，直接把内存吃光。
//!
//! 这里用一个带预算的计数闸门：解码前按「预计占用」预约，用完（闸门守卫析构）释放。
//! 预算内可以多个文件并行；超预算时后来的任务会阻塞等待，而不是把机器拖进交换区。

use std::sync::{Arc, Condvar, Mutex};

/// 默认内存预算：2 GiB。
///
/// 5 分钟立体声的解码占用约 170 MB，2 GiB 能让 8 个左右的文件并行；
/// 又能在只有 8 GB 内存的机器上留出足够余量（长曲目可用 `TUNEFORGE_MEMORY_MB` 调大/调小）。
pub const DEFAULT_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 内存预算环境变量（`TUNEFORGE_MEMORY_MB`，单位 MiB）。
///
/// 内存充足的机器可以调大（例如 3072）让 DSP 路径同时处理更多文件；
/// 内存紧张 / 长曲目多时可以调小，避免同时展开太多整曲缓冲。
pub const MEMORY_ENV: &str = "TUNEFORGE_MEMORY_MB";

/// 从环境变量读内存预算；未设置或非法时返回 `None`（用 [`DEFAULT_BUDGET_BYTES`]）。
pub fn budget_from_env() -> Option<u64> {
    std::env::var(MEMORY_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|mb| *mb > 0)
        .map(|mb| mb.saturating_mul(1024 * 1024))
}

/// 内存闸门（可跨线程共享）。
#[derive(Debug)]
pub struct MemoryGate {
    budget: u64,
    used: Mutex<u64>,
    available: Condvar,
}

impl MemoryGate {
    /// 新建闸门；`budget == 0` 时用 [`DEFAULT_BUDGET_BYTES`]。
    pub fn new(budget: u64) -> Self {
        MemoryGate {
            budget: if budget == 0 {
                DEFAULT_BUDGET_BYTES
            } else {
                budget
            },
            used: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    /// 预算（字节）。
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// 当前已占用（字节）。
    pub fn in_use(&self) -> u64 {
        *self.used.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 预约 `bytes` 字节，返回释放用的守卫。
    ///
    /// * `bytes` 超过总预算时按总预算计（否则会永远等下去）。
    /// * `bytes == 0` 立即返回，不占用预算。
    pub fn acquire(self: &Arc<Self>, bytes: u64) -> MemoryPermit {
        let want = bytes.min(self.budget);
        if want == 0 {
            return MemoryPermit {
                gate: None,
                bytes: 0,
            };
        }
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while *used + want > self.budget {
            used = self.available.wait(used).unwrap_or_else(|e| e.into_inner());
        }
        *used += want;
        MemoryPermit {
            gate: Some(Arc::clone(self)),
            bytes: want,
        }
    }
}

impl Default for MemoryGate {
    fn default() -> Self {
        MemoryGate::new(budget_from_env().unwrap_or(DEFAULT_BUDGET_BYTES))
    }
}

/// 内存预约守卫；析构时自动释放。
#[derive(Debug)]
pub struct MemoryPermit {
    gate: Option<Arc<MemoryGate>>,
    bytes: u64,
}

impl MemoryPermit {
    /// 本次预约的字节数（已按预算截断）。
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for MemoryPermit {
    fn drop(&mut self) {
        let Some(gate) = self.gate.take() else { return };
        let mut used = gate.used.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(self.bytes);
        gate.available.notify_all();
    }
}

/// 估算把整首曲子解码成 `f64` 平面缓冲的占用（字节）。
///
/// 取「采样率 × 声道 × 8 字节 × 2」：一份解码缓冲，外加限幅/增益阶段的一份副本。
/// 采样率缺省按 44.1 kHz，声道缺省按 2，时长上限 2 小时（避免异常元数据放进程阻塞）。
pub fn estimate_buffer_bytes(
    duration_secs: Option<f64>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
) -> u64 {
    let duration = duration_secs.unwrap_or(0.0).clamp(0.0, 7200.0);
    let rate = u64::from(sample_rate.unwrap_or(44_100).max(1));
    let ch = u64::from(channels.unwrap_or(2).max(1));
    let frames = (duration * rate as f64) as u64;
    frames
        .saturating_mul(ch)
        .saturating_mul(8)
        .saturating_mul(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_are_released_on_drop() {
        let gate = Arc::new(MemoryGate::new(1000));
        {
            let _first = gate.acquire(400);
            let _second = gate.acquire(600);
            assert_eq!(gate.in_use(), 1000);
        }
        assert_eq!(gate.in_use(), 0);
    }

    #[test]
    fn oversized_request_is_clamped_to_budget() {
        let gate = Arc::new(MemoryGate::new(1000));
        let permit = gate.acquire(u64::MAX);
        assert_eq!(permit.bytes(), 1000);
        assert_eq!(gate.in_use(), 1000);
    }

    #[test]
    fn zero_budget_falls_back_to_default() {
        let gate = MemoryGate::new(0);
        assert_eq!(gate.budget(), DEFAULT_BUDGET_BYTES);
    }

    #[test]
    fn env_budget_is_optional_and_in_mib() {
        // 这里只验证解析规则（不碰进程环境，避免测试间互相影响）。
        let parse = |v: Option<&str>| -> Option<u64> {
            v.and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|mb| *mb > 0)
                .map(|mb| mb.saturating_mul(1024 * 1024))
        };
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("0")), None);
        assert_eq!(parse(Some("abc")), None);
        assert_eq!(parse(Some(" 512 ")), Some(512 * 1024 * 1024));
    }

    #[test]
    fn concurrent_requests_stay_within_budget() {
        let gate = Arc::new(MemoryGate::new(1000));
        let peak = Arc::new(Mutex::new((0u64, 0usize)));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let gate = Arc::clone(&gate);
                let peak = Arc::clone(&peak);
                scope.spawn(move || {
                    let permit = gate.acquire(300);
                    let mut guard = peak.lock().unwrap();
                    guard.0 = guard.0.max(gate.in_use());
                    guard.1 += 1;
                    drop(guard);
                    drop(permit);
                });
            }
        });
        let (peak_used, finished) = *peak.lock().unwrap();
        assert_eq!(finished, 8, "所有请求都应最终完成");
        assert!(peak_used <= 1000, "超出预算：{peak_used}");
    }

    #[test]
    fn estimate_scales_with_duration_and_channels() {
        let stereo5min = estimate_buffer_bytes(Some(300.0), Some(44_100), Some(2));
        assert_eq!(stereo5min, 44_100 * 300 * 2 * 8 * 2);
        let mono5min = estimate_buffer_bytes(Some(300.0), Some(44_100), Some(1));
        assert_eq!(mono5min, stereo5min / 2);
        assert_eq!(estimate_buffer_bytes(None, None, None), 0);
    }
}
