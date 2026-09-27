# Tuneforge

Windows 桌面音频工具：格式转换、文件名标准化、音量归一化（EBU R128 + 真峰值限幅）、标签与封面编辑。

所有操作只读源文件，结果写入单独的输出目录。同名文件默认跳过。

> Tuneforge is a Windows desktop app for converting audio, applying filename templates, normalizing loudness to EBU R128 with true-peak limiting, and editing tags and cover art. Sources stay untouched; output goes to a separate folder.

界面为中文，浅色主题为默认，顶栏可切换深色。当前发布目标是 Windows（WebView2）。

## 功能

| 页面 | 做什么 |
|------|--------|
| 转换 | FLAC、WAV、AIFF、ALAC、MP3、AAC、Vorbis、Opus。APE 与 DSD 只能作为输入。采样率按目标格式给出可选项 |
| 重命名 | 模板（默认 `{artist} - {title}`）实时预览，并处理 Windows 非法字符 |
| 归一化 | 集成响度对齐到目标 LUFS（默认 −14），可选真峰值上限（默认 −1 dBTP） |
| 标签 | 常用字段、批量查找替换、从文件名反推、封面嵌入 / 替换 / 删除 / 导出 |

## 环境

- Windows 10 或更新版本（需要 WebView2；安装包可下载引导程序）
- [Rust](https://rustup.rs/) 1.77 或更新版本（见 workspace `rust-version`）
- Node.js `^20.19.0` 或 `>=22.12.0`（Vite 7）
- PowerShell 7（`pwsh`），用于下载 FFmpeg

FFmpeg / FFprobe **不在仓库里**。未下载时界面会提示找不到 ffmpeg。

## 开始

在仓库根目录：

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
npm install --prefix app/ui
cargo test --workspace
npm run tauri:dev
```

发布安装包（MSI 与 NSIS）：

```powershell
npm run tauri:build
```

请用 Tauri CLI 构建（`npm run tauri:dev` / `npm run tauri:build`）。它会把前端构建结果嵌进 release。单独执行 `cargo build --release` 时，程序会去打开开发地址 `http://localhost:1420`，开发服务器没启动时窗口是空白页。

仓库根的 `package.json` 只转发到 `app/`。前端工程在 `app/ui`。

更完整的构建、测试、打包路径和运行时约定见 [docs/development.md](docs/development.md)。

## 文档

| 文档 | 内容 |
|------|------|
| [docs/design.md](docs/design.md) | 架构、数据流、DSP 与任务模型。源码注释里的「设计方案 §n」指这里的章节 |
| [docs/contracts/tauri-commands.md](docs/contracts/tauri-commands.md) | 前端调用的命令、事件与类型 |
| [docs/development.md](docs/development.md) | 开发、测试、打包、子进程与启动探测 |
| [docs/audio-quality.md](docs/audio-quality.md) | 转码与归一化链路上已修正的音质问题，以及仍在的限制 |
| [docs/third-party.md](docs/third-party.md) | FFmpeg（LGPL）的获取与分发义务 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 如何改代码、如何提交流程 |

## 仓库结构

```
crates/tf-core     领域逻辑（响度、真峰值、限幅、折混、命名）。无外部 I/O
crates/tf-media    ffmpeg / ffprobe 子进程
crates/tf-tags     标签与封面（lofty）
crates/tf-jobs     任务计划、队列、原子输出、冲突策略
app/src-tauri      Tauri 命令与状态
app/ui             React + TypeScript + Vite + Tailwind CSS
scripts            下载 FFmpeg、生成图标
```

依赖方向：`ui → src-tauri → tf-jobs → { tf-media, tf-tags } → tf-core`。

## 已知限制

- APE 只能作为输入（FFmpeg 没有 APE 编码器）。
- 只把 DSD 解码为 PCM，不做 PCM → DSD。
- 归一化会重编码。需要保持采样不变时，输出用无损格式。
- 取消不会打断已经在跑的那一次 ffmpeg 编码；该文件的结果会被丢弃，源文件不受影响。
- DSD 超声滤波参数已预留，流水线尚未接入，解码行为取决于 ffmpeg。

音质相关的细节与测量记录在 [docs/audio-quality.md](docs/audio-quality.md)。

## 许可

本仓库代码以 **MIT OR Apache-2.0** 双许可发布，见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。

随应用分发的 FFmpeg 是独立的 LGPL 程序，不静态链接进 Tuneforge。说明见 [docs/third-party.md](docs/third-party.md)。
