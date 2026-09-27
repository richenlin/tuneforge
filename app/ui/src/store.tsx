// 全局状态：ffmpeg 定位、媒体列表、选择、任务进度与日志。

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, ApiError, listenFfmpegStatus, listenJobEvents } from "./api";
import {
  markFinished,
  markProgress,
  markStarted,
  newQueueTracker,
  snapshot,
  type JobProgress,
} from "./progress";

export type { JobProgress };
import type {
  ConflictPolicy,
  FfmpegStatus,
  FormatOption,
  JobReport,
  JobRequest,
  JobResult,
  LogEntry,
  LogLevel,
  MeasureRow,
  MediaItem,
  NormalizeParams,
  PageId,
  PolicyOption,
  QueueEvent,
  TemplatePreset,
} from "./types";

export interface MeasureProgress {
  done: number;
  total: number;
}

export interface Toast {
  kind: "info" | "error" | "success";
  text: string;
}

export interface AppContextValue {
  page: PageId;
  setPage: (page: PageId) => void;

  ffmpeg: FfmpegStatus | null;
  ffmpegReady: boolean;
  /** 后台仍在探测 ffmpeg（窗口已可用，但还不能扫描/处理）。 */
  ffmpegProbing: boolean;
  refreshFfmpeg: () => Promise<void>;
  applyFfmpegPath: (path: string) => Promise<void>;
  pickFfmpegDir: () => Promise<void>;

  items: MediaItem[];
  itemsById: Map<string, MediaItem>;
  refreshItems: () => Promise<void>;
  scan: (paths: string[], recursive: boolean) => Promise<void>;
  clearList: () => Promise<void>;
  removeItems: (ids: string[]) => Promise<void>;

  selected: string[];
  toggleSelect: (id: string) => void;
  setSelection: (ids: string[]) => void;
  toggleSelectAll: () => void;
  effectiveIds: string[];

  outputDir: string;
  setOutputDir: (dir: string) => void;
  policy: ConflictPolicy;
  setPolicy: (policy: ConflictPolicy) => void;
  policyOptions: PolicyOption[];
  pickOutputDir: () => Promise<void>;

  formats: FormatOption[];
  presets: TemplatePreset[];
  normalizeDefaults: NormalizeParams | null;

  measurements: Map<string, MeasureRow>;
  measure: (ids: string[]) => Promise<void>;
  measureProgress: MeasureProgress | null;

  jobId: string | null;
  running: boolean;
  progress: JobProgress | null;
  results: JobResult[];
  report: JobReport | null;
  jobError: string | null;
  startJob: (request: JobRequest) => Promise<void>;
  cancelJob: () => Promise<void>;
  exportReport: (format: "csv" | "json") => Promise<void>;
  exportPlan: (request: JobRequest) => Promise<void>;

  logs: LogEntry[];
  clearLogs: () => void;
  toast: Toast | null;
  notify: (kind: Toast["kind"], text: string) => void;
  dismissToast: () => void;

  pickInputPaths: () => Promise<string[]>;
  pickImageFile: () => Promise<string | null>;

  /** 当前功能页注册的请求构造器（底部操作栏使用）。 */
  registerRequestBuilder: (builder: (() => JobRequest | null) | null) => void;
  buildRequest: () => JobRequest | null;
}

const AppContext = createContext<AppContextValue | null>(null);

const LS_OUTPUT = "tuneforge.outputDir";
const LS_POLICY = "tuneforge.policy";

/** 响度测量的并行路数（后端自身是逐个文件顺序测量的）。 */
const MEASURE_LANES = 3;

function errorText(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}

function logLevelOrder(level: LogEntry["level"]): number {
  switch (level) {
    case "error":
      return 0;
    case "warn":
      return 1;
    case "success":
      return 2;
    case "info":
      return 3;
    default:
      return 4;
  }
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [page, setPage] = useState<PageId>("convert");
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus | null>(null);
  const [items, setItems] = useState<MediaItem[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [outputDir, setOutputDirState] = useState<string>(
    () => localStorage.getItem(LS_OUTPUT) ?? "",
  );
  const [policy, setPolicyState] = useState<ConflictPolicy>(() => {
    const stored = localStorage.getItem(LS_POLICY);
    return stored === "overwrite" || stored === "rename" ? stored : "skip";
  });
  const [policyOptions, setPolicyOptions] = useState<PolicyOption[]>([]);
  const [formats, setFormats] = useState<FormatOption[]>([]);
  const [presets, setPresets] = useState<TemplatePreset[]>([]);
  const [normalizeDefaults, setNormalizeDefaults] = useState<NormalizeParams | null>(null);
  const [measurements, setMeasurements] = useState<Map<string, MeasureRow>>(new Map());
  const [measureProgress, setMeasureProgress] = useState<MeasureProgress | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [results, setResults] = useState<JobResult[]>([]);
  const [report, setReport] = useState<JobReport | null>(null);
  const [jobError, setJobError] = useState<string | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [toast, setToast] = useState<Toast | null>(null);
  const toastTimer = useRef<number | null>(null);
  const requestBuilder = useRef<(() => JobRequest | null) | null>(null);

  const registerRequestBuilder = useCallback((builder: (() => JobRequest | null) | null) => {
    requestBuilder.current = builder;
  }, []);

  const buildRequest = useCallback(() => requestBuilder.current?.() ?? null, []);

  const notify = useCallback((kind: Toast["kind"], text: string) => {
    setToast({ kind, text });
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), kind === "error" ? 8000 : 4000);
  }, []);

  const dismissToast = useCallback(() => setToast(null), []);

  const pushLog = useCallback((level: LogEntry["level"], message: string) => {
    setLogs((prev) => {
      const next = [...prev, { level, message, at: Date.now() }];
      return next.length > 500 ? next.slice(next.length - 500) : next;
    });
  }, []);

  // ---------------------------------------------------------------- ffmpeg

  const refreshFfmpeg = useCallback(async () => {
    try {
      // 该命令已改为非阻塞：立即返回快照（可能仍是“检测中”）。
      setFfmpeg(await api.ffmpegStatus());
    } catch (error) {
      setFfmpeg({
        available: false,
        probing: false,
        ffmpegPath: null,
        ffprobePath: null,
        source: null,
        version: null,
        encodable: [],
        unsupported: [],
        message: errorText(error),
        probeMs: null,
        cached: false,
      });
    }
  }, []);

  // 后台探测完成 → 事件回推，无需用户操作
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void listenFfmpegStatus((status) => {
      setFfmpeg(status);
      if (status.available) {
        pushLog(
          "info",
          `FFmpeg 就绪：${status.source ?? "未知来源"}${status.probeMs !== null ? `（${status.probeMs} ms${status.cached ? "，命中缓存" : ""}）` : ""}`,
        );
      } else if (status.message) {
        pushLog("warn", status.message);
      }
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else dispose = unlisten;
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [pushLog]);

  const applyFfmpegPath = useCallback(
    async (path: string) => {
      try {
        const status = await api.setFfmpegPath(path);
        setFfmpeg(status);
        notify("info", "已提交路径，正在后台检测 FFmpeg…");
      } catch (error) {
        notify("error", errorText(error));
      }
    },
    [notify],
  );

  const pickFfmpegDir = useCallback(async () => {
    const picked = await openDialog({ directory: true, multiple: false, title: "选择包含 ffmpeg 的目录" });
    if (typeof picked === "string") await applyFfmpegPath(picked);
  }, [applyFfmpegPath]);

  const pickInputPaths = useCallback(async (): Promise<string[]> => {
    const picked = await openDialog({
      directory: false,
      multiple: true,
      title: "选择音频文件（也可直接拖拽到窗口）",
      filters: [
        {
          name: "音频",
          extensions: [
            "flac","wav","aif","aiff","aifc","m4a","mp4","alac","ape","wv","tta","dsf","dff","mp3","aac","ogg","oga","opus","wma","wv",
          ],
        },
      ],
    });
    if (Array.isArray(picked)) return picked;
    if (typeof picked === "string") return [picked];
    return [];
  }, []);

  const pickImageFile = useCallback(async (): Promise<string | null> => {
    const picked = await openDialog({
      directory: false,
      multiple: false,
      title: "选择封面图片",
      filters: [{ name: "图片", extensions: ["jpg", "jpeg", "png", "webp", "bmp", "gif"] }],
    });
    return typeof picked === "string" ? picked : null;
  }, []);

  // ---------------------------------------------------------------- 列表

  const refreshItems = useCallback(async () => {
    try {
      const list = await api.listItems();
      setItems(list);
    } catch (error) {
      notify("error", errorText(error));
    }
  }, [notify]);

  const scan = useCallback(
    async (paths: string[], recursive: boolean) => {
      if (paths.length === 0) return;
      try {
        const scanned = await api.scanInputs(paths, recursive);
        const fresh = await api.listItems();
        setItems(fresh);
        const failed = scanned.filter((item) => item.error).length;
        notify(
          "success",
          `已扫描 ${scanned.length} 个文件${failed > 0 ? `（${failed} 个探测失败）` : ""}`,
        );
        pushLog("info", `扫描完成：${scanned.length} 个文件${recursive ? "（含子目录）" : ""}`);
      } catch (error) {
        notify("error", errorText(error));
      }
    },
    [notify, pushLog],
  );

  const clearList = useCallback(async () => {
    await api.clearItems();
    setSelected([]);
    setMeasurements(new Map());
    await refreshItems();
  }, [refreshItems]);

  const removeItems = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return;
      const list = await api.removeItems(ids);
      setItems(list);
      setSelected((prev) => prev.filter((id) => !ids.includes(id)));
    },
    [],
  );

  const toggleSelect = useCallback((id: string) => {
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }, []);

  const setSelection = useCallback((ids: string[]) => setSelected(ids), []);

  const toggleSelectAll = useCallback(() => {
    setSelected((prev) => (prev.length === items.length ? [] : items.map((item) => item.id)));
  }, [items]);

  const effectiveIds = useMemo(
    () => (selected.length > 0 ? selected : items.map((item) => item.id)),
    [selected, items],
  );

  const itemsById = useMemo(() => {
    const map = new Map<string, MediaItem>();
    for (const item of items) map.set(item.id, item);
    return map;
  }, [items]);

  // ---------------------------------------------------------------- 输出设置

  const setOutputDir = useCallback((dir: string) => {
    setOutputDirState(dir);
    localStorage.setItem(LS_OUTPUT, dir);
  }, []);

  const setPolicy = useCallback((value: ConflictPolicy) => {
    setPolicyState(value);
    localStorage.setItem(LS_POLICY, value);
  }, []);

  const pickOutputDir = useCallback(async () => {
    const picked = await openDialog({ directory: true, multiple: false, title: "选择输出目录" });
    if (typeof picked === "string") setOutputDir(picked);
  }, [setOutputDir]);

  // ---------------------------------------------------------------- 测量

  const measure = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) {
        notify("info", "请先在左侧列表选择文件");
        return;
      }
      setMeasureProgress({ done: 0, total: ids.length });
      try {
        pushLog("info", `开始测量响度与真峰值…（${ids.length} 个文件）`);
        // 后端逐个文件顺序测量，这里并行跑几路并逐个回报进度，避免长任务无反馈
        const rows: MeasureRow[] = [];
        let done = 0;
        let next = 0;
        const lanes = Math.min(MEASURE_LANES, ids.length);
        await Promise.all(
          Array.from({ length: lanes }, async () => {
            for (;;) {
              const index = next;
              next += 1;
              if (index >= ids.length) return;
              const batch = await api.measureLoudness([ids[index]]);
              rows.push(...batch);
              done += 1;
              setMeasureProgress({ done, total: ids.length });
            }
          }),
        );
        setMeasurements((prev) => {
          const map = new Map(prev);
          for (const row of rows) map.set(row.id, row);
          return map;
        });
        const failed = rows.filter((row) => row.error).length;
        notify(failed > 0 ? "info" : "success", `测量完成：${rows.length} 个文件${failed ? `（${failed} 个失败）` : ""}`);
        for (const row of rows) {
          if (row.error) pushLog("error", `${row.fileName}：${row.error}`);
        }
        await refreshItems();
      } catch (error) {
        notify("error", errorText(error));
      } finally {
        setMeasureProgress(null);
      }
    },
    [notify, pushLog, refreshItems],
  );

  // ---------------------------------------------------------------- 任务

  /** 队列进度累加器：跨文件统计总体进度（每个文件一条任务）。 */
  const queue = useRef(newQueueTracker(0, Date.now()));

  /** 根据累加器发布进度（总体进度只前进、不回退）。 */
  const publishProgress = useCallback(() => {
    setProgress(snapshot(queue.current, Date.now()));
  }, []);

  const handleEvent = useCallback(
    (event: QueueEvent) => {
      const q = queue.current;
      switch (event.event) {
        case "started":
          markStarted(q, event.jobId, event.label, event.input);
          publishProgress();
          break;
        case "progress":
          markProgress(q, event.jobId, event.percent, event.stage);
          publishProgress();
          break;
        case "log":
          pushLog(event.level as LogLevel, event.message);
          break;
        case "finished":
          markFinished(q, event.result.jobId);
          publishProgress();
          setResults((prev) => [...prev, event.result]);
          break;
      }
    },
    [publishProgress, pushLog],
  );

  const handleDone = useCallback(
    (payload: { jobId: string; report?: JobReport; error?: string }) => {
      setRunning(false);
      setProgress(null);
      if (payload.error) {
        setJobError(payload.error);
        notify("error", payload.error);
        pushLog("error", payload.error);
        return;
      }
      if (payload.report) {
        setReport(payload.report);
        const summary = summarize(payload.report.results);
        pushLog(
          summary.failed > 0 ? "warn" : "success",
          `任务结束：成功 ${summary.success} / 跳过 ${summary.skipped} / 失败 ${summary.failed}`,
        );
        notify(
          summary.failed > 0 ? "info" : "success",
          `任务结束：成功 ${summary.success}，跳过 ${summary.skipped}，失败 ${summary.failed}`,
        );
      }
      void refreshItems();
    },
    [notify, pushLog, refreshItems],
  );

  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void listenJobEvents({ onEvent: handleEvent, onDone: handleDone }).then((unlisten) => {
      if (cancelled) unlisten();
      else dispose = unlisten;
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [handleEvent, handleDone]);

  const startJob = useCallback(
    async (request: JobRequest) => {
      if (!outputDir.trim()) {
        notify("error", "请先选择输出目录");
        return;
      }
      setJobError(null);
      setReport(null);
      setResults([]);
      setLogs([]);
      queue.current = newQueueTracker(request.ids.length, Date.now());
      setProgress(null);
      try {
        const id = await api.startJob(request);
        setJobId(id);
        setRunning(true);
        notify("info", "任务已开始");
      } catch (error) {
        notify("error", errorText(error));
        pushLog("error", errorText(error));
      }
    },
    [notify, outputDir, pushLog],
  );

  const cancelJob = useCallback(async () => {
    if (!jobId) return;
    try {
      await api.cancelJob(jobId);
      notify("info", "已请求取消（已完成文件保留）");
      pushLog("warn", "用户请求取消");
    } catch (error) {
      notify("error", errorText(error));
    }
  }, [jobId, notify, pushLog]);

  const exportReport = useCallback(
    async (format: "csv" | "json") => {
      if (!jobId) {
        notify("error", "没有可导出的任务");
        return;
      }
      const path = await saveDialog({
        title: "导出结果",
        defaultPath: `tuneforge-report.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return;
      try {
        const written = await api.exportReport(jobId, format, path);
        notify("success", `已导出：${written}`);
      } catch (error) {
        notify("error", errorText(error));
      }
    },
    [jobId, notify],
  );

  const exportPlan = useCallback(
    async (request: JobRequest) => {
      const path = await saveDialog({
        title: "导出任务清单（可重跑）",
        defaultPath: "tuneforge-plan.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      try {
        const written = await api.exportPlan(request, path);
        notify("success", `任务清单已导出：${written}`);
      } catch (error) {
        notify("error", errorText(error));
      }
    },
    [notify],
  );

  // ---------------------------------------------------------------- 初始化

  useEffect(() => {
    void (async () => {
      await refreshFfmpeg();
      await refreshItems();
      try {
        setPolicyOptions(await api.conflictPolicies());
      } catch {
        setPolicyOptions([
          { id: "skip", label: "跳过" },
          { id: "overwrite", label: "覆盖" },
          { id: "rename", label: "自动重命名" },
        ]);
      }
      try {
        setPresets(await api.templatePresets());
      } catch {
        setPresets([]);
      }
      try {
        setNormalizeDefaults(await api.defaultNormalizeParams());
      } catch {
        setNormalizeDefaults(null);
      }
    })();
  }, [refreshFfmpeg, refreshItems]);

  const ffmpegReady = ffmpeg?.available === true;
  const ffmpegProbing = !ffmpegReady && (ffmpeg === null || ffmpeg.probing);

  useEffect(() => {
    if (!ffmpegReady) return;
    void (async () => {
      try {
        setFormats(await api.encodeOptions());
      } catch (error) {
        notify("error", errorText(error));
      }
    })();
  }, [ffmpegReady, notify]);

  const value: AppContextValue = {
    page,
    setPage,
    ffmpeg,
    ffmpegReady,
    ffmpegProbing,
    refreshFfmpeg,
    applyFfmpegPath,
    pickFfmpegDir,
    items,
    itemsById,
    refreshItems,
    scan,
    clearList,
    removeItems,
    selected,
    toggleSelect,
    setSelection,
    toggleSelectAll,
    effectiveIds,
    outputDir,
    setOutputDir,
    policy,
    setPolicy,
    policyOptions,
    pickOutputDir,
    formats,
    presets,
    normalizeDefaults,
    measurements,
    measure,
    measureProgress,
    jobId,
    running,
    progress,
    results,
    report,
    jobError,
    startJob,
    cancelJob,
    exportReport,
    exportPlan,
    logs,
    clearLogs: () => setLogs([]),
    toast,
    notify,
    dismissToast,
    pickInputPaths,
    pickImageFile,
    registerRequestBuilder,
    buildRequest,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const context = useContext(AppContext);
  if (!context) throw new Error("useApp 必须在 AppProvider 内使用");
  return context;
}

/** 结果统计（前端本地汇总，避免额外 invoke）。 */
export function summarize(results: JobResult[]) {
  return {
    total: results.length,
    success: results.filter((r) => r.status === "success").length,
    skipped: results.filter((r) => r.status === "skipped").length,
    failed: results.filter((r) => r.status === "failed").length,
    cancelled: results.filter((r) => r.status === "cancelled").length,
    bytes: results.reduce((sum, r) => sum + r.bytes, 0),
  };
}

export { logLevelOrder };
