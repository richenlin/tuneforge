// 前后端类型定义，与 `docs/contracts/tauri-commands.md` 保持一致。

export type PageId = "convert" | "rename" | "normalize" | "tags";

export type ConflictPolicy = "skip" | "overwrite" | "rename";
export type MissingFieldPolicy = "empty" | "placeholder" | "skip";
export type ChannelMode = "keep" | "stereo" | "mono";
export type SourceRating = "hi_res" | "lossless" | "lossy" | "dsd";
export type TaskStatus = "success" | "skipped" | "failed" | "cancelled";
export type LogLevel = "error" | "warn" | "info" | "debug";

export type TagField =
  | "title"
  | "artist"
  | "album"
  | "album_artist"
  | "track"
  | "disc"
  | "year"
  | "genre"
  | "comment";

export const TAG_FIELDS: { id: TagField; label: string; numeric: boolean }[] = [
  { id: "title", label: "标题", numeric: false },
  { id: "artist", label: "艺术家", numeric: false },
  { id: "album", label: "专辑", numeric: false },
  { id: "album_artist", label: "专辑艺术家", numeric: false },
  { id: "track", label: "音轨号", numeric: true },
  { id: "disc", label: "碟号", numeric: true },
  { id: "year", label: "年份", numeric: false },
  { id: "genre", label: "流派", numeric: false },
  { id: "comment", label: "备注", numeric: false },
];

export interface Tags {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  albumArtist?: string | null;
  track?: number | null;
  disc?: number | null;
  year?: string | null;
  genre?: string | null;
  comment?: string | null;
}

export interface MediaInfo {
  path: string;
  format: string | null;
  container: string;
  codec: string;
  sampleRate: number | null;
  bitsPerSample: number | null;
  isFloat: boolean;
  channels: number | null;
  channelLayout: string | null;
  durationSecs: number | null;
  frames: number | null;
  bitRate: number | null;
  sizeBytes: number;
}

export interface MediaItem {
  id: string;
  path: string;
  fileName: string;
  media: MediaInfo | null;
  tags: Tags;
  hasCover: boolean;
  error: string | null;
  loudnessLufs: number | null;
  truePeakDbtp: number | null;
  fakeMultichannel: boolean;
  rating: SourceRating | null;
}

export interface FfmpegStatus {
  available: boolean;
  /** 仍在后台探测（UI 显示“检测中”，按钮暂不可用）。 */
  probing: boolean;
  ffmpegPath: string | null;
  ffprobePath: string | null;
  /** 中文来源标签（随包分发 / 用户指定 / 程序目录 / 系统 PATH）。 */
  source: string | null;
  version: string | null;
  encodable: string[];
  unsupported: { format: string; label: string; reason: string }[];
  message: string | null;
  /** 探测耗时（毫秒）。 */
  probeMs: number | null;
  /** 是否命中磁盘缓存（未启动子进程）。 */
  cached: boolean;
}

export interface FormatOption {
  format: string;
  label: string;
  extension: string;
  lossless: boolean;
  available: boolean;
  reason: string | null;
  defaultBitDepth: number | null;
  defaultQuality: string;
}

/** 编码质量参数由后端给出，前端原样回传。 */
export type EncodeQuality = unknown;

export interface EncodeSpec {
  format: string;
  bitDepth: number | null;
  sampleRate: number | null;
  channels: number | null;
  quality: EncodeQuality;
}

/** 采样率下拉项（按目标格式给出；`value === null` 表示保持源采样率）。 */
export interface SampleRateOption {
  value: number | null;
  label: string;
  /** 是否为推荐项（列表里只有一项为 true）。 */
  recommended: boolean;
  /** 对“保持源”项：源采样率是否被目标格式直接支持。 */
  supported: boolean;
}

export interface SanitizeOptions {
  replacement: string;
  collapseSpaces: boolean;
  fullwidthToHalfwidth: boolean;
  maxLen: number;
  fallback: string;
}

export interface NormalizeParams {
  targetLufs: number;
  ceilingDbtp: number | null;
  lookaheadMs: number;
  attackMs: number;
  releaseMs: number;
  downmixFakeMultichannel: boolean;
}

export interface ConvertConfig {
  spec: EncodeSpec;
  keepTags: boolean;
  gainDb: number | null;
  ceilingDbtp: number | null;
  channelMode: ChannelMode;
  downmixFakeMultichannel: boolean;
}

export type JobRequest =
  | {
      page: "convert";
      outputDir: string;
      policy: ConflictPolicy;
      ids: string[];
      config: ConvertConfig;
    }
  | {
      page: "rename";
      outputDir: string;
      policy: ConflictPolicy;
      template: string;
      missing: MissingFieldPolicy;
      sanitize: SanitizeOptions;
      ids: string[];
    }
  | {
      page: "normalize";
      outputDir: string;
      policy: ConflictPolicy;
      ids: string[];
      params: NormalizeParams;
      format: string;
      bitDepth: number | null;
      keepTags: boolean;
    }
  | { page: "tags"; outputDir: string; policy: ConflictPolicy; ids: string[] };

export interface RenameConflict {
  kind: "duplicate_in_batch" | "exists_on_disk" | "same_as_source";
  name?: string;
  path?: string;
}

export interface RenamePreviewItem {
  sourcePath: string;
  originalName: string;
  newName: string;
  changed: boolean;
  skip: boolean;
  notes: string[];
  conflict: RenameConflict | null;
}

export interface TemplatePreset {
  id: string;
  label: string;
  template: string;
}

export interface MeasureRow {
  id: string;
  fileName: string;
  loudnessLufs: number | null;
  truePeakDbtp: number | null;
  samplePeakDbfs: number | null;
  fakeMultichannel: boolean;
  note: string | null;
  error: string | null;
}

export interface GainPreviewRow {
  id: string;
  fileName: string;
  measuredLufs: number | null;
  truePeakDbtp: number | null;
  gainDb: number;
  needsMeasuring: boolean;
}

export interface JobResult {
  jobId: string;
  kind: "convert" | "rename" | "normalize" | "tags";
  input: string;
  inputName: string;
  output: string | null;
  status: TaskStatus;
  error: string | null;
  errorCategory: string | null;
  note: string | null;
  bytes: number;
  elapsedMs: number;
}

export interface Summary {
  total: number;
  success: number;
  skipped: number;
  failed: number;
  cancelled: number;
  bytes: number;
  elapsedMs: number;
}

export interface JobReport {
  outputDir: string;
  results: JobResult[];
  elapsedMs: number;
  concurrency: number;
}

export type QueueEvent =
  | { event: "started"; jobId: string; label: string; input: string }
  | { event: "progress"; jobId: string; percent: number; stage: string }
  | { event: "log"; jobId: string; level: LogLevel; message: string }
  | { event: "finished"; result: JobResult };

export interface PolicyOption {
  id: ConflictPolicy;
  label: string;
}

export interface AppInfo {
  name: string;
  version: string;
  features: string[];
  notes: string[];
}

export interface LogEntry {
  level: LogLevel | "success";
  message: string;
  at: number;
}
