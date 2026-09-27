# FFmpeg 捆绑目录（随包分发）

设计方案 §11：Tuneforge 以子进程调用 FFmpeg，并把可执行文件随安装包分发。可执行文件不进入 git。许可见 [docs/third-party.md](../../../docs/third-party.md)。

## 目录约定

```
app/src-tauri/binaries/ffmpeg-<target-triple>.exe
app/src-tauri/binaries/ffprobe-<target-triple>.exe
```

`target-triple` 例如 `x86_64-pc-windows-msvc`。运行时查找顺序（`tf-media::locate`）：

1. 用户指定路径（界面里的 `set_ffmpeg_path`）
2. `resources/binaries/<triple>` → `resources/binaries`（Tauri `externalBin` 解包位置）
3. 可执行文件目录下的 `binaries/<triple>` → `binaries`
4. 系统 `PATH`

## 获取二进制

运行 `scripts/fetch-ffmpeg.ps1`（PowerShell）：

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
```

脚本会下载 BtbN 的 LGPL 构建，并只保留 `ffmpeg.exe` / `ffprobe.exe`。

安装包请使用默认的 `lgpl` 参数。`app/src-tauri/licenses/FFMPEG-LICENSE.txt` 会随 `bundle.resources` 打进安装包。`.gitignore` 已排除本目录的 `*.exe`。
