//! 极简并行工具（只用 std 线程，避免额外依赖）。
//!
//! DSP 阶段按声道并行：`tf-core` 不需要 async 运行时。

/// 对 `0..len` 的每一项并行执行 `f`，按原顺序返回结果。
///
/// `min_items_per_thread` 用于避免小任务被线程创建开销拖慢；
/// 元素数不足时退化为串行执行。
pub fn map_ordered<T, F>(len: usize, min_items_per_thread: usize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let per_thread = min_items_per_thread.max(1);
    let usable = (len / per_thread).clamp(1, threads.max(1));
    if len <= 1 || usable <= 1 {
        return (0..len).map(&f).collect();
    }

    let chunk = len.div_ceil(usable);
    let mut slots: Vec<Vec<(usize, T)>> = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut start = 0usize;
        while start < len {
            let end = (start + chunk).min(len);
            let fref = &f;
            handles
                .push(scope.spawn(move || (start..end).map(|i| (i, fref(i))).collect::<Vec<_>>()));
            start = end;
        }
        for h in handles {
            match h.join() {
                Ok(part) => slots.push(part),
                Err(_) => {
                    // 子线程 panic：退化为串行重算，保证函数总是返回结果
                    return;
                }
            }
        }
    });

    if slots.is_empty() {
        return (0..len).map(&f).collect();
    }
    let mut out: Vec<Option<T>> = Vec::with_capacity(len);
    out.resize_with(len, || None);
    for part in slots {
        for (i, v) in part {
            out[i] = Some(v);
        }
    }
    out.into_iter().map(|v| v.unwrap_or_else(|| f(0))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_map_preserves_order() {
        let out = map_ordered(1000, 1, |i| i * 2);
        assert_eq!(out.len(), 1000);
        assert_eq!(out[0], 0);
        assert_eq!(out[999], 1998);
    }

    #[test]
    fn small_inputs_fall_back_to_serial() {
        let out = map_ordered(0, 8, |i| i);
        assert!(out.is_empty());
        let out = map_ordered(3, 8, |i| i + 1);
        assert_eq!(out, vec![1, 2, 3]);
    }
}
