# 更新记录

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [0.1.0] - 2026-09-27

首次公开源码。

### 性能

- 批量转换并发度从硬编码 `min(CPU 核心数, 4)` 改为 `min(CPU 核心数, 16)`，可用 `TUNEFORGE_CONCURRENCY` 覆盖；
  16 线程机器上实测比旧的 4 并发快 1.5–1.8 倍（见 `docs/design.md` §10.5）。
- 每个 ffmpeg 子进程按 `max(1, 核心数 / 并发度)` 限制线程数（`-threads`），避免多进程互相抢 CPU。
- 新增内存闸门：DSP 路径（归一化等）按预算（默认 2 GiB，`TUNEFORGE_MEMORY_MB` 可调）预约整曲解码内存，
  高并发不再把内存吃爆。
- 新增开发用并发基线基准：`cargo run -p tf-jobs --release --example parallel_bench`。

### 其他

- 拖入多个文件时会锁定界面并显示扫描进度（`app/ui/src/components/BusyOverlay.tsx`）。
- 任务结束后台面板显示本次实际并发度。

- Windows 桌面应用：格式转换、文件名标准化、EBU R128 音量归一化（真峰值限幅）、标签与封面编辑。
- 源文件只读，结果写入独立输出目录。
- FFmpeg / FFprobe 作为 LGPL sidecar 由脚本下载，不包含在 git 历史中。
