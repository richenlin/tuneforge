# scripts

| 脚本 | 用途 |
|------|------|
| `fetch-ffmpeg.ps1` | 下载 LGPL 版 ffmpeg / ffprobe，按 Tauri `externalBin` 命名放到 `app/src-tauri/binaries/` |
| `generate-icons.mjs` | 生成 `app/src-tauri/icons/`（32 / 128 / 256 PNG 与 `icon.ico`） |

许可与再分发见 [docs/third-party.md](../docs/third-party.md)。

## ffmpeg sidecar

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
pwsh -File scripts/fetch-ffmpeg.ps1 -Version 8.1 -Flavor lgpl
```

默认是 FFmpeg **8.1**、`lgpl`、`x86_64-pc-windows-msvc`。`latest` 发布里的 zip 名会变，脚本通过 GitHub API 解析资产；API 失败时回退到固定文件名。

`app/src-tauri/binaries/*.exe` 不入库。`tf-media::locate` 拒绝 0 字节文件，因此没跑过脚本时，界面会提示未找到 ffmpeg。

安装包分发请保持 `-Flavor lgpl`。

## 图标

```bash
node scripts/generate-icons.mjs
```

Windows 上 `tauri-build` 需要 `icon.ico` 才能生成资源。仓库里已经有一套图标，只有改徽标时才要重新跑。
