# 开发、测试与运行时约定

面向改代码的人。使用者从 [README](../README.md) 的「开始」即可构建。

## 环境与依赖

| 工具 | 版本 |
|------|------|
| Windows | 10 或更新，带 WebView2 |
| Rust | 1.77+（`Cargo.toml` 的 `rust-version`） |
| Node.js | `^20.19.0` 或 `>=22.12.0` |
| PowerShell | 7（`pwsh`） |

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
npm install --prefix app/ui
npm install --prefix app
```

`fetch-ffmpeg.ps1` 默认：FFmpeg 8.1、`lgpl`、`x86_64-pc-windows-msvc`。脚本说明见 [scripts/README.md](../scripts/README.md)。

仓库根目录：

```powershell
npm run tauri:dev      # 开发窗口
npm run tauri:build    # MSI + NSIS
npm run typecheck      # 只检查前端类型
cargo test --workspace
```

这些 npm 脚本转发到 `app/`。真正的前端工程在 `app/ui`。

### 为什么用 Tauri CLI 构建

`npm run tauri:build` 会先构建 `app/ui/dist`，再让 `tauri-build` 在 release 里嵌入前端。单独 `cargo build --release` 缺少这套环境变量时，按开发模式去加载 `devUrl`（`http://localhost:1420`）。那台机器上没有 Vite 时，窗口是空白页。

### 打包产物

在仓库根执行 `npm run tauri:build` 之后：

| 路径 | 说明 |
|------|------|
| `target/release/tuneforge.exe` | 主程序（内嵌前端与图标） |
| `target/release/ffmpeg.exe`、`ffprobe.exe` | 打进包内的 sidecar |
| `target/release/bundle/msi/Tuneforge_0.1.0_x64_en-US.msi` | MSI |
| `target/release/bundle/nsis/Tuneforge_0.1.0_x64-setup.exe` | NSIS |

版本号来自 `app/src-tauri/tauri.conf.json`。文件名里的 `0.1.0` 会随版本变化。

图标在 `app/src-tauri/icons/`。要换徽标时执行 `node scripts/generate-icons.mjs`。缺少 `icon.ico` 时，Windows 上的 `tauri-build` 无法生成资源文件，`cargo build` 会失败。

## 测试

| 层次 | 覆盖 |
|------|------|
| `tf-core` | 响度、真峰值、限幅收敛、抖动、折混、模板渲染与合法化 |
| `tf-media` | 参数拼装、进度解析、扫描规则、sidecar 定位 |
| `tf-tags` | 标签往返、封面增删改查、导出 |
| `tf-jobs` | 计划、并发度、冲突策略、原子替换、报告导出 |
| `tuneforge`（命令层） | id 稳定性、错误 JSON、状态、封面暂存 |

`tf-core` 不依赖 ffmpeg，合成信号测试不需要 sidecar。需要真实解码的路径才用本机的 ffmpeg。

## 运行时约定

### ffmpeg 探测不占 UI 线程

`state::AppState::start_probe()` 在 `setup` 里启动后台线程做定位和能力查询。`ffmpeg_status` 先返回 `probing: true`，完成后再用 `ffmpeg:status` 事件推送。

结果写入 `%LOCALAPPDATA%/<identifier>/cache/ffmpeg-capabilities.json`，用 ffmpeg / ffprobe 的文件大小加 mtime 作指纹。指纹命中则不再启动子进程。

`scan_inputs`、`measure_loudness`、`preview_decode` 是 `async`，重活放在 `spawn_blocking` 里。

下面是一次开发机测量，用来说明这个取舍，不是持续维护的基准：

| 场景 | 主线程同步探测 | 后台探测 + 缓存 |
|------|----------------|-----------------|
| 窗口出现到首次 IPC 返回 | 11330 ms | 11 ms |
| 首次探测（无缓存） | 阻塞界面 | 6.4 s，界面可操作 |
| 再次启动 | 重新启动两次进程 | 0 ms（命中缓存） |

### 子进程不要弹出控制台

FFmpeg / FFprobe 是控制台程序。Windows 上用 `std::process::Command::new` 直接启动时，每次 spawn 会多一个控制台窗口；默认终端是 Windows Terminal 时就是一串半透明窗口。

所有子进程走 `tf_media::process::command()`。它在 Windows 上设置 `CREATE_NO_WINDOW`。

| 写法 | 一次 spawn 新增控制台 | 能否捕获输出 |
|------|----------------------|--------------|
| `std::process::Command::new` | 1 | 能 |
| `tf_media::process::command` | 0 | 能 |

打包版上扫过 3 个 FLAC 并转成 MP3（大约 9 次 spawn），新增控制台窗口为 0。

## 端到端核对记录（2026-09-26）

对当时的 `target/release/tuneforge.exe`（内嵌前端，没有开发服务器）从命令层跑过：

1. `scan_inputs`：2 个 WAV（44.1 kHz / 16-bit / 2 ch，以及 48 kHz / 16-bit / 1 ch），探测并读标签。
2. `measure_loudness`：`01 tone.wav` → −21.75 LUFS，真峰值 −21.07 dBTP。
3. `start_job`（转换）：输出 FLAC 24-bit / 44.1 kHz / 2 ch，`TITLE` / `ARTIST` / `ALBUM` / `track` 保留。
4. `start_job`（归一化，目标 −14 LUFS / −1 dBTP）：2 个文件都成功。再次测量输出为 −14.000 LUFS，真峰值 −13.32 与 −10.09 dBTP。

这是当时构建的记录，不是自动回归。音质链路的另外几项核对在 [audio-quality.md](audio-quality.md)。
