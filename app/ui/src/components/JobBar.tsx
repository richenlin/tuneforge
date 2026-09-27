// 底部控制台：输出目录 / 冲突策略 / 开始-取消 / 进度 / 结果 / 日志抽屉。

import { useEffect, useMemo, useRef, useState } from "react";
import type { ConflictPolicy, JobRequest, LogEntry } from "../types";
import { useApp, type JobProgress } from "../store";
import { api } from "../api";
import {
  Badge,
  Button,
  Divider,
  IconButton,
  Progress,
  Segmented,
  Stat,
  STATUS_LABEL,
  TextInput,
  formatBytes,
  formatDuration,
} from "./ui";
import { Icon } from "./Icons";

type LogFilter = "all" | "warn" | "error";

function LogDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { logs, clearLogs } = useApp();
  const [filter, setFilter] = useState<LogFilter>("all");
  const endRef = useRef<HTMLDivElement | null>(null);

  const visible = useMemo(() => {
    if (filter === "all") return logs;
    if (filter === "warn") return logs.filter((entry) => entry.level === "warn");
    return logs.filter((entry) => entry.level === "error");
  }, [logs, filter]);

  useEffect(() => {
    if (open) endRef.current?.scrollIntoView({ block: "end" });
  }, [visible, open]);

  if (!open) return null;

  const tone = (entry: LogEntry) => {
    switch (entry.level) {
      case "error":
        return "text-rose-ink";
      case "warn":
        return "text-amber-ink";
      case "success":
        return "text-mint";
      case "debug":
        return "text-ink-3";
      default:
        return "text-ink-2";
    }
  };

  return (
    <div className="border-t border-line bg-surface-2">
      <div className="flex items-center gap-2 px-4 py-2">
        <Icon name="terminal" size={14} className="text-ink-3" />
        <span className="text-[12px] font-semibold text-ink">运行日志</span>
        <Badge>{logs.length}</Badge>
        <div className="ml-2 flex items-center gap-1">
          {(
            [
              ["all", "全部"],
              ["warn", "警告"],
              ["error", "错误"],
            ] as [LogFilter, string][]
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              onClick={() => setFilter(value)}
              className={`rounded-md px-2 py-[3px] text-[11px] transition-colors ${
                filter === value ? "bg-surface-3 text-ink" : "text-ink-3 hover:text-ink"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="ml-auto flex items-center gap-1.5">
          <Button size="sm" variant="ghost" icon="trash" onClick={clearLogs}>
            清空
          </Button>
          <IconButton icon="x" title="收起日志" size="sm" onClick={onClose} />
        </div>
      </div>
      <div className="max-h-40 overflow-auto px-4 pb-3 font-mono text-[11px] leading-[1.75]">
        {visible.length === 0 ? (
          <div className="py-2 text-ink-3">暂无日志</div>
        ) : (
          visible.map((entry, index) => (
            <div key={`${entry.at}-${index}`} className="flex gap-2">
              <span className="tnum shrink-0 text-ink-3">
                {new Date(entry.at).toLocaleTimeString("zh-CN", { hour12: false })}
              </span>
              <span className={`min-w-0 whitespace-pre-wrap break-all ${tone(entry)}`}>{entry.message}</span>
            </div>
          ))
        )}
        <div ref={endRef} />
      </div>
    </div>
  );
}

export function JobBar({
  buildRequest,
  startLabel = "开始处理",
}: {
  buildRequest: () => JobRequest | null;
  startLabel?: string;
}) {
  const {
    outputDir,
    setOutputDir,
    pickOutputDir,
    policy,
    setPolicy,
    running,
    progress,
    startJob,
    cancelJob,
    results,
    report,
    jobError,
    exportReport,
    exportPlan,
    effectiveIds,
    items,
    notify,
  } = useApp();
  const [logsOpen, setLogsOpen] = useState(false);
  const [resultsOpen, setResultsOpen] = useState(false);

  const selectedCount = effectiveIds.length;

  const onStart = async () => {
    const request = buildRequest();
    if (!request) return;
    await startJob(request);
    setResultsOpen(true);
  };

  const stats = report ? summarize(report.results) : null;
  const live = summarize(results);

  const openOutputDir = async () => {
    if (!outputDir) return;
    try {
      await api.openInExplorer(outputDir);
    } catch (error) {
      notify("error", error instanceof Error ? error.message : String(error));
    }
  };

  const canStart = items.length > 0 && selectedCount > 0 && outputDir.trim().length > 0;

  return (
    <div className="shrink-0 border-t border-line bg-surface/85 backdrop-blur-md">
      <div className="flex flex-wrap items-end gap-3 px-4 pt-3">
        <div className="min-w-[300px] flex-1">
          <span className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-ink-2">
            <Icon name="folder" size={12} className="text-ink-3" />
            输出目录
          </span>
          <div className="flex gap-2">
            <TextInput value={outputDir} onChange={setOutputDir} placeholder="例如 D:\\音乐\\输出" mono />
            <Button icon="folder" onClick={() => void pickOutputDir()}>
              选择…
            </Button>
            <IconButton icon="external" title="在资源管理器中打开" disabled={!outputDir} onClick={() => void openOutputDir()} />
          </div>
        </div>

        <div>
          <span className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-ink-2">
            <Icon name="layers" size={12} className="text-ink-3" />
            同名冲突
          </span>
          <Segmented
            value={policy}
            onChange={(value) => setPolicy(value as ConflictPolicy)}
            options={[
              { value: "skip", label: "跳过" },
              { value: "overwrite", label: "覆盖" },
              { value: "rename", label: "自动重命名" },
            ]}
          />
        </div>

        <Divider vertical />

        <div className="flex items-center gap-2">
          {running ? (
            <Button variant="danger" icon="stop" onClick={() => void cancelJob()}>
              取消任务
            </Button>
          ) : (
            <Button
              variant="primary"
              icon="play"
              disabled={!canStart}
              onClick={() => void onStart()}
              title={canStart ? startLabel : "请先扫描并选择文件，并设置输出目录"}
            >
              {startLabel}
            </Button>
          )}
          <ExportMenu
            reportReady={report !== null}
            planReady={items.length > 0}
            onExport={(format) => void exportReport(format)}
            onExportPlan={() => {
              const request = buildRequest();
              if (request) void exportPlan(request);
            }}
          />
          <IconButton icon="terminal" title="运行日志" size="sm" tone={logsOpen ? "accent" : "default"} onClick={() => setLogsOpen((v) => !v)} />
        </div>
      </div>

      <div className="px-4 pb-3 pt-2.5">
        {running ? (
          <div className="flex flex-col gap-2">
            <Progress
              value={progress?.percent ?? 0}
              label={progress?.input || "准备中…"}
              detail={progressDetail(progress)}
            />
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-ink-3">
              {live.success > 0 && <span className="text-mint">成功 {live.success}</span>}
              {live.skipped > 0 && <span className="text-amber-ink">跳过 {live.skipped}</span>}
              {live.failed > 0 && <span className="text-rose-ink">失败 {live.failed}</span>}
              {progress !== null && progress.etaSecs !== null && (
                <span className="flex items-center gap-1.5">
                  <Icon name="clock" size={12} />
                  预计剩余 {formatDuration(progress.etaSecs)}
                </span>
              )}
            </div>
          </div>
        ) : stats ? (
          <div className="flex flex-wrap items-center gap-2">
            <Stat label="成功" value={stats.success} tone="ok" icon="checkCircle" />
            <Stat label="跳过" value={stats.skipped} tone="warn" icon="refresh" />
            <Stat label="失败" value={stats.failed} tone={stats.failed > 0 ? "err" : "neutral"} icon="xCircle" />
            {stats.cancelled > 0 && <Stat label="取消" value={stats.cancelled} icon="stop" />}
            <Stat label="输出" value={formatBytes(stats.bytes)} icon="download" />
            {report && <Stat label="耗时" value={formatDuration(report.elapsedMs / 1000)} icon="clock" />}
            <button
              type="button"
              onClick={() => setResultsOpen((v) => !v)}
              className="ml-auto inline-flex items-center gap-1 text-[11.5px] text-accent hover:underline"
            >
              {resultsOpen ? "隐藏明细" : "查看明细"}
              <Icon name={resultsOpen ? "chevronDown" : "chevronRight"} size={12} />
            </button>
          </div>
        ) : jobError ? (
          <div className="flex items-center gap-2 text-[12px] text-rose-ink">
            <Icon name="alert" size={14} />
            {jobError}
          </div>
        ) : (
          <div className="flex items-center gap-2 text-[11.5px] text-ink-3">
            <Icon name="info" size={12} />
            {selectedCount > 0 ? (
              <span>
                将对 <span className="tnum font-mono text-ink-2">{selectedCount}</span> 个文件执行
                「{startLabel.replace("开始", "")}」
                {outputDir.trim() ? "" : "，请先选择输出目录"}
              </span>
            ) : (
              "请先扫描并选择要处理的文件"
            )}
          </div>
        )}
      </div>

      {resultsOpen && (
        <div className="max-h-52 overflow-auto border-t border-line bg-surface-2/70 px-4 py-2.5">
          {results.length === 0 ? (
            <div className="py-3 text-center text-[12px] text-ink-3">暂无处理结果</div>
          ) : (
            <table className="w-full border-collapse">
              <thead className="text-left text-[10.5px] uppercase tracking-wider text-ink-3">
                <tr>
                  <th className="px-2 py-1.5">文件</th>
                  <th className="w-20 px-2 py-1.5">状态</th>
                  <th className="px-2 py-1.5">输出</th>
                  <th className="w-20 px-2 py-1.5 text-right">大小</th>
                  <th className="w-16 px-2 py-1.5 text-right">耗时</th>
                  <th className="px-2 py-1.5">说明</th>
                </tr>
              </thead>
              <tbody>
                {results.map((result) => {
                  const status = STATUS_LABEL[result.status] ?? { text: result.status, tone: "neutral" as const };
                  return (
                    <tr key={result.jobId} className="border-t border-line">
                      <td className="max-w-[240px] truncate px-2 py-1.5 text-[11.5px] text-ink" title={result.input}>
                        {result.inputName}
                      </td>
                      <td className="px-2 py-1.5">
                        <Badge tone={status.tone}>{status.text}</Badge>
                      </td>
                      <td className="max-w-[260px] truncate px-2 py-1.5 font-mono text-[11px] text-ink-3" title={result.output ?? ""}>
                        {result.output ?? "—"}
                      </td>
                      <td className="tnum px-2 py-1.5 text-right font-mono text-[11px] text-ink-2">
                        {formatBytes(result.bytes)}
                      </td>
                      <td className="tnum px-2 py-1.5 text-right font-mono text-[11px] text-ink-2">
                        {formatDuration(result.elapsedMs / 1000)}
                      </td>
                      <td className="px-2 py-1.5 text-[11px] text-rose-ink/80">{result.error ?? result.note ?? ""}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      )}

      <LogDrawer open={logsOpen} onClose={() => setLogsOpen(false)} />
    </div>
  );
}

function ExportMenu({
  reportReady,
  planReady,
  onExport,
  onExportPlan,
}: {
  reportReady: boolean;
  planReady: boolean;
  onExport: (format: "csv" | "json") => void;
  onExportPlan: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const items: [string, boolean, () => void][] = [
    ["导出 CSV 结果", reportReady, () => onExport("csv")],
    ["导出 JSON 结果", reportReady, () => onExport("json")],
    ["导出任务清单（可重跑）", planReady, onExportPlan],
  ];

  return (
    <div className="relative" ref={ref}>
      <IconButton
        icon="download"
        title="导出"
        size="sm"
        tone={open ? "accent" : "default"}
        disabled={!reportReady && !planReady}
        onClick={() => setOpen((value) => !value)}
      />
      {open && (
        <div className="absolute bottom-9 right-0 z-30 w-48 overflow-hidden rounded-xl border border-line bg-surface py-1 shadow-[var(--shadow-pop)]">
          {items.map(([label, enabled, action]) => (
            <button
              key={label}
              type="button"
              disabled={!enabled}
              onClick={() => {
                setOpen(false);
                action();
              }}
              className="block w-full px-3 py-2 text-left text-[12px] text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            >
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function progressDetail(progress: JobProgress | null): string {
  if (!progress) return "准备中…";
  const parts: string[] = [];
  if (progress.total > 0) {
    parts.push(`第 ${Math.min(progress.done + 1, progress.total)} / ${progress.total} 个`);
  }
  if (progress.stage) {
    parts.push(
      progress.filePercent > 0 ? `${progress.stage} ${Math.round(progress.filePercent * 100)}%` : progress.stage,
    );
  }
  return parts.join(" · ") || "处理中…";
}

function summarize(rows: { status: string; bytes: number }[]) {
  return {
    success: rows.filter((r) => r.status === "success").length,
    skipped: rows.filter((r) => r.status === "skipped").length,
    failed: rows.filter((r) => r.status === "failed").length,
    cancelled: rows.filter((r) => r.status === "cancelled").length,
    bytes: rows.reduce((sum, r) => sum + r.bytes, 0),
  };
}
