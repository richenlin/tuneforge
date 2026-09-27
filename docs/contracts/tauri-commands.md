# Tuneforge 前后端契约（v0.1）

前端通过 Tauri `invoke` 调用后端命令，通过事件接收进度。所有命令出错时返回 `{ message, category }` 形状的字符串错误（见 `error_dto`）。

字段名以本文件和 `app/src-tauri` 的序列化为准。文档索引见 [../README.md](../README.md)。

## 命令一览

| 命令 | 参数（camelCase） | 返回 | 说明 |
|------|-------------------|------|------|
| `ffmpeg_status` | 无 | `FfmpegStatus` | ffmpeg/ffprobe 定位结果、版本、可用输出格式 |
| `set_ffmpeg_path` | `path: string` | `FfmpegStatus` | 覆盖 ffmpeg 目录或可执行文件路径（设计方案 §11） |
| `scan_inputs` | `paths: string[]`, `recursive: boolean` | `MediaItem[]` | 扫描 + ffprobe + lofty 读取标签，登记到内存列表 |
| `list_items` | 无 | `MediaItem[]` | 当前列表快照 |
| `clear_items` | 无 | `null` | 清空列表 |
| `remove_items` | `ids: string[]` | `MediaItem[]` | 从列表移除 |
| `encode_options` | 无 | `FormatOption[]` | 转换页格式下拉（含推荐默认值与不可用原因） |
| `default_encode_spec` | `format: string`, `itemId?: string` | `EncodeSpec` | 按 §6.2 推荐参数（采样率固定为“保持源”） |
| `sample_rate_options` | `format: string`, `itemId?: string` | `SampleRateOption[]` | 按**目标格式**给出采样率候选与推荐项（UI 下拉，禁止用户手填） |
| `measure_loudness` | `ids: string[]` | `MeasureRow[]` | 逐文件 LUFS/真峰值/假多声道检测 |
| `preview_rename` | `template: string`, `missing: "empty"\|"placeholder"\|"skip"`, `sanitize: SanitizeOptions`, `outputDir: string`, `ids?: string[]` | `RenamePreviewItem[]` | 实时预览 + 冲突检测（不落盘） |
| `template_presets` | 无 | `TemplatePreset[]` | 重命名预设 |
| `start_job` | `request: JobRequest` | `string`（jobId） | 生成任务清单并在后台执行 |
| `cancel_job` | `jobId: string` | `null` | 取消（已完成文件保留） |
| `job_report` | `jobId: string` | `JobReport` | 结果汇总 |
| `export_report` | `jobId: string`, `format: "csv"\|"json"`, `path: string` | `string`（写入路径） | 导出结果（§12） |
| `export_plan` | `request: JobRequest`, `path: string` | `string` | 导出任务清单 JSON（§10.4 可重跑） |
| `update_tags` | `ids: string[]`, `edits: FieldEdit[]` | `MediaItem[]` | 只改内存中的标签（批量编辑） |
| `replace_in_tags` | `ids: string[]`, `field: string`, `find: string`, `replace: string`, `caseInsensitive: boolean` | `MediaItem[]` | 批量查找替换 |
| `guess_tags` | `ids: string[]`, `template: string` | `MediaItem[]` | 从文件名反推标签 |
| `cover_preview` | `itemId: string` | `string \| null` | data URL（base64） |
| `set_cover` | `itemId: string`, `imagePath: string` | `string \| null` | 嵌入/替换封面 |
| `remove_cover` | `itemId: string` | `null` | 删除封面 |
| `export_cover` | `itemId: string`, `index: number`, `outDir: string` | `string` | 导出封面 |
| `open_in_explorer` | `path: string` | `null` | 打开输出目录 |
| `app_info` | 无 | `AppInfo` | 应用名 / 版本 / 功能与注意事项（关于页） |
| `conflict_policies` | 无 | `PolicyOption[]` | 冲突策略下拉（中文标签） |
| `default_normalize_params` | 无 | `NormalizeParams` | 归一化默认参数（设计方案 D8） |
| `gain_preview` | `ids: string[]`, `params: NormalizeParams` | `GainPreviewRow[]` | 归一化页的增益预估（不重新解码） |
| `preview_decode` | `itemId: string`, `seconds?: number` | `DecodePreview` | 调试用：解码至多 N 秒并返回响度/真峰值 |

> 所有结构体字段在 JSON 中均为 **camelCase**（Rust 侧 `#[serde(rename_all = "camelCase")]`）；
> 命令参数名在 JS 侧写 camelCase，Tauri 会自动映射到 Rust 的 snake_case 参数。

## 事件

`listen("job:event")` 收到 `QueueEvent`：

```ts
type QueueEvent =
  | { event: "started";  jobId: string; label: string; input: string }
  | { event: "progress"; jobId: string; percent: number; stage: string }
  | { event: "log";      jobId: string; level: "debug"|"info"|"warn"|"error"; message: string }
  | { event: "finished"; result: JobResult };
```

`listen("job:done")` 收到 `{ jobId: string; report: JobReport }`（`report.concurrency` = 本次实际并发度，用于界面显示）。

`listen("ffmpeg:status")` 收到 `FfmpegStatus`：窗口先出图，ffmpeg 后台探测完再回推。

`listen("scan:progress")` 收到 `{ done: number; total: number }`：`scan_inputs` 每探测完一个文件回推一次。
多文件拖入时扫描可能持续数十秒，前端据此锁定界面并显示进度（`app/ui/src/components/BusyOverlay.tsx`）。

## 关键类型

```ts
interface FfmpegStatus {
  available: boolean;
  ffmpegPath: string | null;
  ffprobePath: string | null;
  source: "override" | "bundled_sidecar" | "executable_dir" | "path" | null;
  version: string | null;
  encodable: string[];                                     // AudioFormat id
  unsupported: { format: string; label: string; reason: string }[];
  message: string | null;
}

interface MediaItem {
  id: string; path: string; fileName: string;
  media: MediaInfo | null;
  tags: Tags;
  hasCover: boolean;
  error: string | null;
  loudnessLufs: number | null;
  truePeakDbtp: number | null;
  fakeMultichannel: boolean;
  rating: "hi_res" | "lossless" | "lossy" | "dsd" | null;
}

interface Tags { title?; artist?; album?; albumArtist?; track?: number; disc?: number; year?; genre?; comment? } // 全部 string | null

interface FormatOption {
  format: string; label: string; extension: string; lossless: boolean;
  available: boolean; reason: string | null;
  defaultBitDepth: number | null;
  defaultQuality: string;      // 中文摘要，例如 "FLAC 压缩级别 8"
}

interface EncodeSpec { format: string; bitDepth: number | null; sampleRate: number | null; channels: number | null; quality: EncodeQuality }

type JobRequest =
  | { page: "convert";   outputDir: string; policy: ConflictPolicy; ids: string[]; config: ConvertConfig }
  | { page: "rename";    outputDir: string; policy: ConflictPolicy; template: string; missing: MissingFieldPolicy; sanitize: SanitizeOptions; ids: string[] }
  | { page: "normalize"; outputDir: string; policy: ConflictPolicy; ids: string[]; params: NormalizeParams; format: string; bitDepth: number | null; keepTags: boolean }
  | { page: "tags";      outputDir: string; policy: ConflictPolicy; ids: string[] };

type ConflictPolicy = "skip" | "overwrite" | "rename";       // 默认 skip
interface FieldEdit { field: TagField; value: string | null } // value 为 null 表示清除该字段
interface PolicyOption { id: ConflictPolicy; label: string }
interface SampleRateOption { value: number | null; label: string; recommended: boolean; supported: boolean }
interface GainPreviewRow { id: string; fileName: string; measuredLufs: number | null; truePeakDbtp: number | null; gainDb: number; needsMeasuring: boolean }
interface AppInfo { name: string; version: string; features: string[]; notes: string[] }
interface DecodePreview { sampleRate: number; channels: number; frames: number; loudnessLufs: number | null; truePeakDbtp: number }
interface ConvertConfig { spec: EncodeSpec; keepTags: boolean; gainDb: number | null; ceilingDbtp: number | null; channelMode: "keep"|"stereo"|"mono"; downmixFakeMultichannel: boolean }
interface NormalizeParams { targetLufs: number; ceilingDbtp: number | null; lookaheadMs: number; attackMs: number; releaseMs: number; downmixFakeMultichannel: boolean }
interface SanitizeOptions { replacement: string; collapseSpaces: boolean; fullwidthToHalfwidth: boolean; maxLen: number; fallback: string }
```

## UI 约定

### 采样率（转换页）

采样率**不允许手填**，一律由后端按目标格式给出下拉项与推荐值（`sample_rate_options`）：

| 目标格式 | 可选采样率 | 说明 |
|----------|-----------|------|
| FLAC / WAV / AIFF / ALAC | 44.1k / 48k / 88.2k / 96k / 176.4k / 192k / 384k | 无损：源采样率被支持时**推荐保持源**（不重采样） |
| MP3 | 32k / 44.1k / 48k | 上限 48 kHz；源 96 kHz 时自动推荐 48 kHz |
| AAC | 32k / 44.1k / 48k / 88.2k / 96k | 上限 96 kHz |
| OGG Vorbis | 44.1k / 48k / 88.2k / 96k / 192k | 上限 192 kHz |
| Opus | **仅 48 kHz** | 编码器硬约束；其他源采样率会被重采样，界面会提示 |

* 列表首项固定为“保持源采样率（<源> Hz）”，源采样率不被目标格式支持时该项会标注“当前格式不支持，将被重采样”，并把推荐项自动选中。
* 需要重采样时，推荐“不高于源采样率的最大候选值”，尽量保留信息量。
* 后端会对 `start_job` 的 `spec.sampleRate` 做校验，越界返回中文错误（例：`Opus 不支持 44 100 Hz 采样率（最大 48 000 Hz）`）。

- 顶部：输入选择（拖拽 / 文件夹选择）+ 扫描按钮；中部：文件列表；右侧：参数面板；底部：输出目录、冲突策略、开始/取消、总进度、日志抽屉（设计方案 §6.1）。
- 四个独立功能页：转换 / 重命名 / 归一化 / 标签（中文优先，文案为中文）。
- 输出目录必填，且不能等于源目录（后端会拒绝并给出中文提示）。
- 未定位到 ffmpeg 时，页面需显示阻断提示与“设置 ffmpeg 路径”入口。
