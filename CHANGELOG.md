# 更新记录

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [0.1.0] - 2026-09-27

首次公开源码。

- Windows 桌面应用：格式转换、文件名标准化、EBU R128 音量归一化（真峰值限幅）、标签与封面编辑。
- 源文件只读，结果写入独立输出目录。
- FFmpeg / FFprobe 作为 LGPL sidecar 由脚本下载，不包含在 git 历史中。
