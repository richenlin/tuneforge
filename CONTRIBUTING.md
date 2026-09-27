# 参与贡献

感谢你改进 Tuneforge。本文件说明本地怎么跑起来、改动怎么提交，以及代码以什么许可进入仓库。

## 开发环境

与 [README](README.md) 的「环境」相同：Windows、Rust 1.77+、Node.js `^20.19.0` 或 `>=22.12.0`、PowerShell 7。

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
npm install --prefix app/ui
npm install --prefix app
cargo test --workspace
npm run tauri:dev
```

`app/` 只提供 Tauri CLI。前端依赖装在 `app/ui`。

改图标时运行 `node scripts/generate-icons.mjs`。日常构建不需要重新生成，`app/src-tauri/icons/` 已在仓库中。

## 提交前

按你改动的范围运行：

| 改动 | 命令 |
|------|------|
| 任意 Rust | `cargo test --workspace` |
| `app/ui` | `npm run typecheck`（仓库根目录即可） |
| 编码、归一化、标签写入 | 再用少量真实文件走一遍对应页面，确认源文件未被改写 |

新增 ffmpeg / ffprobe 进程时，走 `tf_media::process::command()`。在 Windows 上直接 `std::process::Command::new` 会为每次启动弹出控制台窗口。说明见 [docs/development.md](docs/development.md)。

设计文档的章节号被源码注释引用。调整行为时，同步更新 [docs/design.md](docs/design.md) 的对应节，并保持「§n」编号不变。命令或 JSON 字段变化时，同步更新 [docs/contracts/tauri-commands.md](docs/contracts/tauri-commands.md)。

## 不要提交的内容

- `app/src-tauri/binaries/*.exe`（FFmpeg / FFprobe，已被 `.gitignore` 排除）
- `target/`、`node_modules/`、`app/ui/dist/`、`app/src-tauri/gen/`
- `.env` 与任何密钥、令牌、个人音乐库路径

应用仓库需要锁定依赖：请提交根目录 `Cargo.lock` 和 `app/ui/package-lock.json`。

## 许可

对本仓库的贡献以与现有代码相同的 **MIT OR Apache-2.0** 双许可提供。提交即表示你有权如此授权。

不要把 GPL 组件链进默认构建。随包分发的 FFmpeg 保持 `scripts/fetch-ffmpeg.ps1` 的默认 `lgpl` 构建。义务见 [docs/third-party.md](docs/third-party.md)。

## 拉取请求

- 一个 PR 解决一件事，说明用户能观察到的行为变化。
- 写清你怎么验证的（测试命令，或手工步骤）。
- 提交说明用完整句子，重点写原因。中文或英文都可以。

## 发布安装包时

1. 运行 `pwsh -File scripts/fetch-ffmpeg.ps1`（保持 `-Flavor lgpl`）。
2. 记下实际下载的 zip 文件名，写进该版本的发布说明，并给出对应源码位置（见 [docs/third-party.md](docs/third-party.md)）。
3. 确认安装包带上 `app/src-tauri/licenses/FFMPEG-LICENSE.txt`（`tauri.conf.json` 的 `bundle.resources` 已包含 `licenses/*`）。
4. `npm run tauri:build`。
5. 在一台没有开发服务器的机器上启动安装后的程序，确认窗口不是空白页，且能定位到捆绑的 ffmpeg。
