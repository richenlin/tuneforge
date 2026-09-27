# 第三方组件

Tuneforge 自己的代码以 MIT OR Apache-2.0 双许可发布。Rust crate 与 npm 包各自保留上游许可证，由 `Cargo.lock` 与 `app/ui/package-lock.json` 锁定版本。

需要单独说明的是 **FFmpeg / FFprobe**。

## FFmpeg

Tuneforge 把 FFmpeg 当作独立程序调用（子进程），不把 FFmpeg 静态链接进 `tuneforge.exe`。

| 项 | 约定 |
|----|------|
| 许可证 | GNU LGPL 2.1 或更高版本。全文：<https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html> |
| 推荐构建 | [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) 的 **lgpl** 压缩包（`scripts/fetch-ffmpeg.ps1` 的默认 `-Flavor lgpl`） |
| 上游源码 | <https://ffmpeg.org/download.html> ，git：<https://git.ffmpeg.org/ffmpeg.git> |
| 本仓库 | **不提交** `ffmpeg.exe` / `ffprobe.exe`。`.gitignore` 排除 `app/src-tauri/binaries/*.exe` |

获取：

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
```

脚本从 BtbN 的 `latest` 发布里挑选 `ffmpeg-n<版本>-latest-win64-lgpl-*.zip`（默认主版本 8.1），只把 `ffmpeg.exe` 与 `ffprobe.exe` 放到 `app/src-tauri/binaries/`，并加上 Tauri `externalBin` 要求的目标三元组后缀。

`latest` 会移动。做安装包时，把当时下载的 **zip 文件名** 写进该版本的发布说明，并给出对应源码（BtbN 该次构建所对应的 FFmpeg 源码，或上游 tag）。这样接收安装包的人能够取得对应源码，满足 LGPL 对可执行文件分发的要求。

安装包通过 `tauri.conf.json` 的 `bundle.resources` 带上 `app/src-tauri/licenses/`。其中 [FFMPEG-LICENSE.txt](../app/src-tauri/licenses/FFMPEG-LICENSE.txt) 是给最终用户看的说明，并指向 LGPL 全文。

### 不要随包分发 GPL 构建

`fetch-ffmpeg.ps1 -Flavor gpl` 会下载带 `--enable-gpl` 组件的构建。那种构建的分发义务和本仓库的 MIT OR Apache-2.0 不是同一套。默认安装包请保持 `lgpl`。

用户如果在自己的机器上把 ffmpeg 路径指到别处，那是用户自己的副本，不由本仓库再分发。
