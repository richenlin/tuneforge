# Tuneforge 桌面音频处理工具 — 设计方案

- 版本：v0.1
- 日期：2026-09-26
- 状态：已按本文实现。与代码不一致时以代码为准。
- 源码注释里的「设计方案 §n」指本文章节。改正文时保持这些编号不变。

文档索引见 [README.md](README.md)。FFmpeg 分发见 [third-party.md](third-party.md)。音质修正的落地记录见 [audio-quality.md](audio-quality.md)。

---

## 1. 背景与目标

本项目把一次真实的音频库整理过程（548 首无损音乐）产品化为桌面工具，
覆盖四个能力：**格式转换、文件名标准化、音量归一化、标签修改**。

核心目标：

1. 支持 `DSD(DFF/DSF)`、`WAV`、`FLAC`、`APE`、`MP3`、`M4A`、`OGG` 等格式互相转换，
   参数默认给出推荐值，且可定制。
2. 文件名标准化：可配置模板、实时预览。
3. 音量归一化（无损重编码）：改音频样本，保持无损格式，可选真峰值限幅。
4. 标签属性修改：常用字段 + 封面。
5. **非破坏性**：所有转换/修改都输出到用户指定的新文件夹，源文件永不改动。

---

## 2. 已确认决策

| 编号 | 决策点 | 结论 |
|------|--------|------|
| D1 | 音频引擎 | 捆绑 FFmpeg 可执行文件，Rust 负责编排与 DSP |
| D2 | 桌面 UI | Tauri（Rust 后端 + Web 前端），Windows 优先 |
| D3 | 归一化"无损"含义 | 修改音频样本并重编码为无损格式（非只写标签） |
| D4 | 文件名标准化 | 可配置模板 + 实时预览，默认 `{artist} - {title}` |
| D5 | 功能组织 | 独立功能页（转换 / 重命名 / 归一化 / 标签） |
| D6 | DSD 处理 | 只做解码：`DSF/DFF → PCM(FLAC/WAV)`，不做 PCM → DSD |
| D7 | 标签编辑范围 | 常用字段 + 封面（嵌入/替换/删除/导出） |
| D8 | 归一化默认参数 | `-14 LUFS` 预设，真峰值 `-1 dBTP`，可定制 |
| D9 | 输出与冲突 | 仅输出到新文件夹；同名默认跳过，可选覆盖/重命名 |
| D10 | 架构 | FFmpeg 子进程 + Rust 编排（方案 A） |
| D11 | 项目名 | `tuneforge` |
| D12 | 前端栈 | React + TypeScript + TailwindCSS |
| D13 | 界面语言 | 中文优先，架构预留 i18n（暂不做英文翻译） |

---

## 3. 需求分解

### R1 格式转换
- 输入：FLAC / WAV / APE / MP3 / M4A / OGG / DSD(DSF/DFF) / AIFF 等（FFmpeg 可解码者）。
- 输出：FLAC / WAV / AIFF / ALAC(M4A) / MP3 / AAC(M4A) / OGG Vorbis / Opus。
- 每个目标格式有**推荐默认参数**，并提供高级自定义。
- 转换时保留标签（可开关）。

> ⚠️ **限制**：FFmpeg 没有 APE 编码器，因此 **APE 只能作为输入**，不能作为输出格式。
> PCM → DSD 不在范围内（见 D6）。

### R2 文件名标准化
- 模板变量：`{artist} {title} {album} {albumartist} {track} {disc} {year} {genre}`。
- 内置预设 + 自定义模板。
- 字符合法化：Windows 非法字符 `\ / : * ? " < > |`、结尾空格/点、多余空格、全半角。
- 改名前列出"原名 → 新名"预览，冲突检测。

### R3 音量归一化（无损）
- EBU R128 / ITU-R BS.1770-4 集成响度测量。
- 目标预设：`-23`（广播）、`-16`（流媒体）、`-14`（默认）、自定义。
- 真峰值上限默认 `-1 dBTP`（可关）。
- 可选把"假多声道"（8 声道但仅前 2 声道有声）折混为立体声。
- 输出保持无损格式（默认 FLAC 24-bit）。

### R4 标签修改
- 字段：`title / artist / album / albumartist / track / disc / year(date) / genre / comment`。
- 封面：嵌入、替换、删除、导出。
- 批量查找替换、从文件名反推标签。

---

## 4. 总体架构

### 4.1 工程结构（Cargo workspace）

下列是模块职责，不是完整文件清单。

```
tuneforge/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ tf-core/                 # 领域逻辑，无外部 I/O 依赖
│  │   ├─ model.rs             # 媒体信息、标签、任务模型
│  │   ├─ loudness.rs          # EBU R128 集成响度
│  │   ├─ truepeak.rs          # 4x 过采样真峰值
│  │   ├─ limiter.rs           # look-ahead 真峰值限幅 + 线性 trim
│  │   ├─ downmix.rs           # 折混
│  │   ├─ naming.rs            # 模板渲染 / 预览 / 合法化
│  │   └─ error.rs             # 错误模型
│  ├─ tf-media/                # ffmpeg / ffprobe 子进程封装
│  │   ├─ probe.rs             # 探测媒体信息
│  │   ├─ decode.rs            # 解码为 PCM
│  │   ├─ encode.rs            # 编码
│  │   └─ locate.rs            # 定位 ffmpeg / ffprobe
│  ├─ tf-tags/                 # 标签与封面 (lofty 封装)
│  │   ├─ read.rs / write.rs
│  │   └─ cover.rs
│  └─ tf-jobs/                 # 任务队列与输出
│      ├─ queue.rs             # 并发池 / 进度 / 取消
│      ├─ output.rs            # 临时文件 / 原子替换 / 冲突策略
│      └─ report.rs            # 结果统计与日志
├─ app/
│  ├─ src-tauri/               # Rust 后端：commands / state / events
│  │  ├─ tauri.conf.json
│  │  ├─ binaries/             # ffmpeg / ffprobe sidecar（不入库）
│  │  └─ licenses/             # 随安装包分发的第三方许可说明
│  └─ ui/                      # 前端（4 个独立功能页）
├─ scripts/                    # 下载 FFmpeg、生成图标
└─ docs/
```

### 4.2 依赖方向

```
ui  →  src-tauri  →  tf-jobs  →  { tf-media, tf-tags }  →  tf-core
```

`tf-core` 不依赖 `tf-media` / `tf-tags`，可用合成信号做纯单元测试。

### 4.3 统一数据流

```
选择输入(文件夹/文件)
   → 扫描 + 探测 (ffprobe + lofty)
   → 列表 / 预览
   → 配置参数
   → 生成任务列表
   → 队列并发执行
        单文件：解码(ffmpeg→PCM) → DSP(Rust) → 编码(ffmpeg) → 写标签(lofty) → 原子移动到输出目录
   → 进度事件 / 日志 / 结果汇总
```

---

## 5. 技术栈

| 层 | 选型 |
|----|------|
| 壳 | Tauri v2 |
| 前端 | React + TypeScript + Vite + TailwindCSS + Radix/shadcn 组件 |
| 音频 I/O | FFmpeg / FFprobe 子进程（随包分发） |
| 标签/封面 | `lofty` |
| 响度 | `ebur128`（或自实现 BS.1770-4）+ 4x 过采样真峰值 |
| 重采样 | FFmpeg `soxr`（或 Rust `rubato`） |
| 并发 | `tokio`（异步任务）+ `rayon`（DSP 并行） |
| 序列化 | `serde` / `serde_json` |
| 错误 | `thiserror`（库）/ `anyhow`（应用层） |
| 日志 | `tracing` + 滚动文件日志 |
| 图片 | `image` |

---

## 6. 功能页设计

### 6.1 公共布局
- 顶部：输入选择（拖拽文件夹/文件）、扫描按钮。
- 中部：文件列表（列：文件名、格式、采样率、位深、声道、时长、LUFS、真峰值、状态）。
- 右侧：该页参数面板。
- 底部：输出目录、冲突策略、开始/取消、总进度条、日志抽屉。

### 6.2 转换页
- 选择目标格式（下拉），参数面板按格式动态切换。
- 默认参数（推荐值）：

| 目标 | 默认参数 |
|------|----------|
| FLAC | 保持源位深（16 或 24；未知或高于 24 则为 24），保留源采样率，压缩级别 8 |
| WAV/AIFF | 源为 16-bit 则 16-bit，否则 24-bit；保留源采样率 |
| ALAC(M4A) | 与 FLAC 相同的位深规则，保留源采样率 |
| MP3 | VBR V0（约 245 kbps） |
| AAC(M4A) | VBR 约 256 kbps |
| OGG Vorbis | 质量 q6（约 192 kbps） |
| Opus | 160 kbps |

采样率默认保持源（`sample_rate: null`）。可选采样率与各格式上限见 [contracts/tauri-commands.md](contracts/tauri-commands.md)。

- 可改：位深、采样率、声道（保持 / 立体声 / 单声道）、可选增益、是否保留标签。

### 6.3 重命名页
- 模板输入框 + 变量插入按钮 + 预设下拉。
- 实时"原名 → 新名"两列预览；非法字符标红并给出替换建议。
- 冲突检测：目标名重复、目标已存在。
- 只重命名（不转码），输出到新文件夹。

### 6.4 归一化页
- 目标响度预设：`-23 / -16 / -14（默认）/ 自定义`。
- 真峰值上限：默认 `-1 dBTP`，可关闭。
- 选项：假多声道折混、输出位深（界面默认 24-bit）、目标格式（界面默认 FLAC）。
- 列表显示测量得到的 LUFS / 真峰值 / 预计增益。

### 6.5 标签页
- 编辑区：常用字段 + 封面缩略图。
- 批量操作：设置同一字段、查找替换、从文件名反推。
- 封面：嵌入（选择图片）、替换、删除、导出。

---

## 7. 音频处理管线（DSP 细节）

### 7.1 探测
- `ffprobe -show_streams -show_format -print_format json` 获取：codec、采样率、位深、
  声道数、时长、采样数、容器标签。
- 用 `lofty` 读取标准标签与封面。

### 7.2 解码
- `ffmpeg -i in -f f32le -acodec pcm_f32le -` 输出 32-bit float 原始 PCM（交织），
  Rust 侧统一转为 `(channels, samples)` 的 `f64` 平面缓冲。
- 32-bit FLAC / APE / DSD 由 FFmpeg 统一处理，规避库差异。

### 7.3 响度与真峰值测量
- 集成响度：ITU-R BS.1770-4（K-weighting + 门限），目标即对该值归一。
- 真峰值：4 倍过采样（polyphase，Kaiser β≈8.6），逐块取峰值。
- **注**：响度必须在**折混之后的立体声信号**上测量，保证结果与实际输出一致。

### 7.4 增益与真峰值限幅（复刻已验证算法）

令目标 `T`（默认 -14 LUFS）、天花板 `C = 10^(-1/20) ≈ 0.891251`。

1. 计算基础增益：`g = 10^((T - L) / 20)`，`L` 为实测集成响度。
2. 计算真峰值包络 `tp`（对 `x·g` 做 4x 过采样取包络）。
3. 若 `max(tp) ≤ C`：不做限幅，输出 `x·g`。
4. 否则做 look-ahead 限幅：
   - 需求增益 `req = min(1, C / tp)`；
   - 前视 `req_la = min_filter(req, 3 ms)`（负 origin 实现前视）；
   - 平滑：`attack 1 ms` + `release 40 ms` 的滑动平均（dB 域）；
   - 限制曲线 `curve = min(smoothed, req_la)`（保证 `curve ≤ req`）。
5. 输出 `y = x · g · curve`。
6. **最终安全 trim**：用"重叠分块真峰值扫描"精确测量 `y` 的真峰值 `tp_out`；
   若 `tp_out > C`，令 `trim = C / tp_out` 并对整首做线性衰减后重新编码
   （线性增益对真峰值是线性缩放，一次即可收敛）。

> 要点：单纯靠"逐点包络×增益"不足以保证真峰值，因为时变增益会破坏
> "增益在重建滤波器窗口内近似恒定"的假设；最终线性 trim 是可靠兜底。

### 7.5 假多声道折混
- 检测：声道数 = 8 且第 3–8 声道静音（RMS 低于阈值）。
- 处理：只取前 L/R 两声道（不做普通 downmix，避免相位/增益引入）。
- 开关默认关闭，列表中以"假 7.1"标记提示。

### 7.6 重采样与抖动
- 默认保留源采样率；仅在用户显式选择或 DSD 解码时重采样。
- DSD → PCM：默认 88.2 kHz（44.1k 家族），可选 176.4 kHz，加低通抗混叠。
- 降位深（如 32→24）使用 TPDF 抖动；升位深不加抖动。

### 7.7 编码
- 通过 FFmpeg 子进程编码，显式指定 codec / 位深 / 采样率 / 比特率。
- 归一化页默认输出 24-bit FLAC（位深与格式都可改）。
- 不写入 ReplayGain 标签：改的是样本，不是回放增益标签。

---

## 8. 标签与封面

- 读取/写入用 `lofty`，跨格式映射：
  - FLAC：Vorbis Comment + PICTURE 块。
  - MP3：ID3v2（TIT2/TPE1/TALB/TPE2/TRCK/TPOS/TDRC/TCON/COMM + APIC）。
  - M4A：iTunes atoms（©nam/©ART/©alb/aART/trkn/disk/©day/©gen/covr）。
  - OGG/Opus：Vorbis Comment + METADATA_BLOCK_PICTURE。
- 封面：嵌入/替换/删除/导出；转换时可选择保留或丢弃。
- 标签写入失败不影响音频文件完整性（先写音频，再写标签，失败仅告警）。

---

## 9. 文件名模板

- 语法：`{变量}` 占位；`{{`/`}}` 转义花括号。
- 缺失字段策略：留空 / 用占位符 / 跳过该文件（可配置）。
- 合法化：替换非法字符 → `_`；压缩连续空格；去除结尾空格与点；长度限制。
- 预览与冲突检测在内存中完成，确认后才生成任务。

---

## 10. 安全、输出与任务队列

### 10.1 非破坏性
- 所有操作**只读源文件**，结果写入用户选定的输出目录。
- 不提供原地修改。

### 10.2 临时文件与原子替换
- 在输出目录写临时文件（如 `.tf-tmp-<uuid>.flac`），完成后 `rename` 为最终名。
- 失败/取消时删除临时文件。

### 10.3 冲突策略
- 默认：跳过已存在文件。
- 可选：覆盖 / 自动重命名（追加 ` (1)`）。

### 10.4 取消与断点
- 每个文件独立任务，可取消；已完成的文件保留。
- 任务清单可导出/重跑。

### 10.5 并发
- 默认并发 = `min(CPU 核心数, 4)`（FFmpeg 本身可能多线程，避免过载）。
- 全局进度 = 完成文件数 / 总数；单文件进度用 FFmpeg `-progress` 解析。

---

## 11. FFmpeg 集成与分发

- 位置：`app/src-tauri/binaries/ffmpeg-<target-triple>.exe` 与 `ffprobe-<target-triple>.exe`。由 `scripts/fetch-ffmpeg.ps1` 下载，不提交进 git。
- Tauri `externalBin` 打包；启动时优先用捆绑版本，找不到再回退 PATH。运行时顺序见 `app/src-tauri/binaries/README.md`。
- 采用 **LGPL** 构建（默认 BtbN FFmpeg-Builds 的 lgpl 包），以独立子进程调用，不静态链接。分发义务见 [third-party.md](third-party.md)。
- 可用设置里的路径覆盖捆绑位置（命令 `set_ffmpeg_path`）。

---

## 12. 错误模型与日志

- 分类：输入错误、不支持的格式、探测失败、解码失败、编码失败、标签失败、磁盘/权限、取消。
- 每个文件任务产出结构化结果（成功/跳过/失败 + 原因），汇总到结果页并可导出 CSV/JSON。
- 日志：`tracing` 分级 + 滚动文件（含每一步的 ffmpeg 命令行，便于排错）。

---

## 13. 测试策略

- **单元测试（tf-core）**：响度（标准测试信号）、真峰值、限幅收敛性、模板渲染、合法化、折混。
- **采样级回归**：对样本文件做"输出 vs 期望"的残差校验（阈值 `< -100 dBFS`），
  复刻本次 548 首验证中使用的方法，作为限幅/增益/折混的正确性金标准。
- **集成测试**：小语料端到端（各格式→各格式），校验采样数/采样率/声道/位深/标签。
- **FFmpeg 可用性测试**：缺失/版本不符时的降级与报错。
- **UI 冒烟**：Tauri 命令层（invoke）用 mock 任务测试。

---

## 14. 里程碑

| 阶段 | 内容 |
|------|------|
| M1 | workspace 骨架 + `tf-media` 探测 + 扫描 |
| M2 | `tf-core` 响度 + 真峰值 + 限幅 + 单测 |
| M3 | `tf-tags` 标签/封面 + `tf-core` 命名模板 |
| M4 | `tf-jobs` 队列 + 原子输出 + 冲突策略 |
| M5 | Tauri 四个功能页 + 进度/取消 |
| M6 | FFmpeg 捆绑 + 打包 + 许可 |
| M7 | 用大规模语料端到端验证 |

v0.1 已落地 M1–M6。M7 的样本不在仓库里；一次小规模打包版核对见 [development.md](development.md)。

---

## 15. 风险与开放问题

1. **APE 不能编码**：FFmpeg 无 APE 编码器，APE 仅作输入（需向用户明示）。
2. **DSD 解码质量**：DSD→PCM 的低通/去噪策略需实测调优。
3. **FFmpeg 许可与分发**：源码仓库不包含二进制。发布安装包时固定所用构建并提供对应源码，见 [third-party.md](third-party.md)。
4. **真峰值限幅复杂度**：已验证算法可复刻，但需完备的采样级回归测试。
5. **性能/内存**：大 192kHz 长曲目与高并发下的资源占用。
6. **Tauri 依赖 WebView2**：Win10+ 通常内置，需处理缺失场景。
7. **位深策略（已定）**：转换到无损格式时默认保持源位深（16 或 24；更高或未知则为 24），只在降位深时做 TPDF 抖动。见 §6.2 与 [audio-quality.md](audio-quality.md)。

### 已确认（原待确认项）
- 项目名：`tuneforge`（D11）。
- 前端技术栈：React + TypeScript + TailwindCSS（D12）。
- 界面语言：中文优先，架构预留 i18n，暂不做英文翻译（D13）。
