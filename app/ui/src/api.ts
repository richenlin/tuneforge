// Tauri 命令封装（契约：`docs/contracts/tauri-commands.md`）。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  ConflictPolicy,
  EncodeSpec,
  FfmpegStatus,
  FormatOption,
  GainPreviewRow,
  JobReport,
  JobRequest,
  MeasureRow,
  MediaItem,
  NormalizeParams,
  PolicyOption,
  QueueEvent,
  RenamePreviewItem,
  SampleRateOption,
  SanitizeOptions,
  TagField,
  TemplatePreset,
} from "./types";

/** 后端错误（字符串错误是 JSON：`{ message, category, categoryLabel }`）。 */
export class ApiError extends Error {
  readonly category: string | null;
  readonly categoryLabel: string | null;

  constructor(message: string, category: string | null = null, categoryLabel: string | null = null) {
    super(message);
    this.name = "ApiError";
    this.category = category;
    this.categoryLabel = categoryLabel;
  }
}

function toApiError(raw: unknown): ApiError {
  if (raw instanceof ApiError) return raw;
  if (typeof raw === "string") {
    try {
      const parsed = JSON.parse(raw) as {
        message?: string;
        category?: string;
        categoryLabel?: string;
      };
      if (parsed && typeof parsed.message === "string") {
        return new ApiError(parsed.message, parsed.category ?? null, parsed.categoryLabel ?? null);
      }
    } catch {
      // 不是 JSON，按纯文本处理
    }
    return new ApiError(raw);
  }
  if (raw instanceof Error) return new ApiError(raw.message);
  return new ApiError(String(raw));
}

async function cmd<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (error) {
    throw toApiError(error);
  }
}

export interface MissingFieldPolicyArg {
  missing: "empty" | "placeholder" | "skip";
}

export const api = {
  ffmpegStatus: () => cmd<FfmpegStatus>("ffmpeg_status"),
  setFfmpegPath: (path: string) => cmd<FfmpegStatus>("set_ffmpeg_path", { path }),

  scanInputs: (paths: string[], recursive: boolean) =>
    cmd<MediaItem[]>("scan_inputs", { paths, recursive }),
  listItems: () => cmd<MediaItem[]>("list_items"),
  clearItems: () => cmd<null>("clear_items"),
  removeItems: (ids: string[]) => cmd<MediaItem[]>("remove_items", { ids }),

  encodeOptions: () => cmd<FormatOption[]>("encode_options"),
  defaultEncodeSpec: (format: string, itemId?: string) =>
    cmd<EncodeSpec>("default_encode_spec", { format, itemId: itemId ?? null }),
  /** 采样率下拉（按目标格式给候选 + 推荐，`value = null` = 保持源）。 */
  sampleRateOptions: (format: string, itemId?: string) =>
    cmd<SampleRateOption[]>("sample_rate_options", { format, itemId: itemId ?? null }),
  measureLoudness: (ids: string[]) => cmd<MeasureRow[]>("measure_loudness", { ids }),
  gainPreview: (ids: string[], params: NormalizeParams) =>
    cmd<GainPreviewRow[]>("gain_preview", { ids, params }),
  defaultNormalizeParams: () => cmd<NormalizeParams>("default_normalize_params"),
  conflictPolicies: () => cmd<PolicyOption[]>("conflict_policies"),

  templatePresets: () => cmd<TemplatePreset[]>("template_presets"),
  previewRename: (args: {
    template: string;
    missing: MissingFieldPolicyArg["missing"];
    sanitize: SanitizeOptions;
    outputDir: string;
    ids: string[];
  }) => cmd<RenamePreviewItem[]>("preview_rename", args),

  updateTags: (ids: string[], edits: { field: TagField; value: string | null }[]) =>
    cmd<MediaItem[]>("update_tags", { ids, edits }),
  replaceInTags: (args: {
    ids: string[];
    field: TagField;
    find: string;
    replace: string;
    caseInsensitive: boolean;
  }) => cmd<MediaItem[]>("replace_in_tags", args),
  guessTags: (ids: string[], template: string) => cmd<MediaItem[]>("guess_tags", { ids, template }),

  coverPreview: (itemId: string) => cmd<string | null>("cover_preview", { itemId }),
  setCover: (itemId: string, imagePath: string) =>
    cmd<string | null>("set_cover", { itemId, imagePath }),
  removeCover: (itemId: string) => cmd<null>("remove_cover", { itemId }),
  exportCover: (itemId: string, index: number, outDir: string) =>
    cmd<string>("export_cover", { itemId, index, outDir }),

  startJob: (request: JobRequest) => cmd<string>("start_job", { request }),
  cancelJob: (jobId: string) => cmd<null>("cancel_job", { jobId }),
  jobReport: (jobId: string) => cmd<JobReport>("job_report", { jobId }),
  exportReport: (jobId: string, format: "csv" | "json", path: string) =>
    cmd<string>("export_report", { jobId, format, path }),
  exportPlan: (request: JobRequest, path: string) => cmd<string>("export_plan", { request, path }),

  openInExplorer: (path: string) => cmd<null>("open_in_explorer", { path }),
  appInfo: () => cmd<AppInfo>("app_info"),
};

/** 订阅任务事件；返回取消订阅函数。 */
export async function listenJobEvents(handlers: {
  onEvent: (event: QueueEvent) => void;
  onDone: (payload: { jobId: string; report?: JobReport; error?: string }) => void;
}): Promise<UnlistenFn> {
  const unlistenEvent = await listen<QueueEvent>("job:event", (e) => handlers.onEvent(e.payload));
  const unlistenDone = await listen<{ jobId: string; report?: JobReport; error?: string }>(
    "job:done",
    (e) => handlers.onDone(e.payload),
  );
  return () => {
    unlistenEvent();
    unlistenDone();
  };
}

/** 订阅后台 ffmpeg 探测结果（探测在线程里跑，完成后回推）。 */
export async function listenFfmpegStatus(
  onStatus: (status: FfmpegStatus) => void,
): Promise<UnlistenFn> {
  return listen<FfmpegStatus>("ffmpeg:status", (e) => onStatus(e.payload));
}

export type { ConflictPolicy };
