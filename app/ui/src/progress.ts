// 队列进度聚合（纯函数，便于单独验证）：
// 后端按「一个文件一条任务」上报进度，这里跨文件聚合成总体进度。

export interface JobProgress {
  /** 任务名（如「格式转换」）。 */
  label: string;
  /** 当前正在处理的文件。 */
  input: string;
  /** 队列总体进度 0..1（跨文件统计，且单调不回退）。 */
  percent: number;
  /** 当前文件自身的进度 0..1（解码/编码阶段各自从 0 开始）。 */
  filePercent: number;
  /** 当前阶段（解码 / 编码 / 写标签…）。 */
  stage: string;
  /** 已完成文件数。 */
  done: number;
  /** 本次任务的文件总数。 */
  total: number;
  /** 预计剩余秒数；进度太少无法估算时为 null。 */
  etaSecs: number | null;
}

/** 队列累加器状态（可在 store 的 ref 中原地更新）。 */
export interface QueueTracker {
  total: number;
  done: number;
  active: Map<string, number>;
  peak: number;
  startedAt: number;
  label: string;
  input: string;
  stage: string;
  filePercent: number;
}

/** 进度太低时不给 ETA 估算，避免出现离谱数字。 */
const MIN_ETA_PERCENT = 0.02;

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

/** 新建累加器；`total` 为本次任务的文件数。 */
export function newQueueTracker(total: number, now: number): QueueTracker {
  return {
    total,
    done: 0,
    active: new Map(),
    peak: 0,
    startedAt: now,
    label: "",
    input: "",
    stage: "准备中",
    filePercent: 0,
  };
}

/** 某个文件开始处理。 */
export function markStarted(tracker: QueueTracker, jobId: string, label: string, input: string): void {
  tracker.active.set(jobId, 0);
  tracker.label = label;
  tracker.input = input;
  tracker.stage = "开始";
  tracker.filePercent = 0;
}

/** 某个文件上报进度。 */
export function markProgress(tracker: QueueTracker, jobId: string, percent: number, stage: string): void {
  const value = clamp01(percent);
  tracker.active.set(jobId, value);
  tracker.filePercent = value;
  tracker.stage = stage;
}

/** 某个文件结束（成功 / 跳过 / 失败 / 取消一律计入已完成）。 */
export function markFinished(tracker: QueueTracker, jobId: string): void {
  tracker.active.delete(jobId);
  tracker.done += 1;
}

/** 生成给 UI 的进度快照：总体进度取历史最高值，单文件内部阶段回退不会让进度条倒退。 */
export function snapshot(tracker: QueueTracker, now: number): JobProgress {
  const total = Math.max(tracker.total, 1);
  let inFlight = 0;
  for (const value of tracker.active.values()) inFlight += value;
  const raw = Math.min(1, (tracker.done + inFlight) / total);
  tracker.peak = Math.max(tracker.peak, raw);
  const elapsedSecs = Math.max(0, (now - tracker.startedAt) / 1000);
  const etaSecs =
    tracker.peak > MIN_ETA_PERCENT ? Math.max(0, (elapsedSecs * (1 - tracker.peak)) / tracker.peak) : null;
  return {
    label: tracker.label,
    input: tracker.input,
    percent: tracker.peak,
    filePercent: tracker.filePercent,
    stage: tracker.stage,
    done: tracker.done,
    total: tracker.total,
    etaSecs,
  };
}
