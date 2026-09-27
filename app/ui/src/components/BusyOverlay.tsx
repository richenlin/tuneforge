// 全局加载遮罩：扫描/探测期间锁定界面并给出明确反馈（设计方案 §6.1）。
//
// 背景：拖入多个文件后要逐个调用 ffprobe 探测，耗时可能从几秒到几十秒。
// 此前这段时间界面没有任何提示、也能继续操作，用户会以为拖放没生效。

import { useEffect, useState } from "react";
import { Progress, Spinner } from "./ui";

export function BusyOverlay({
  title,
  hint,
  count,
  progress,
}: {
  /** 主文案，例如「正在扫描文件…」。 */
  title: string;
  /** 补充说明（为什么慢、要不要等）。 */
  hint?: string;
  /** 本轮提交的项目数（拖入的路径/文件夹数量）。 */
  count?: number;
  /** 逐文件进度（后端 `scan:progress`）；`total` 为 0 表示还没拿到总数。 */
  progress?: { done: number; total: number } | null;
}) {
  // 已等待秒数：只要界面没变，用户至少能看到「还在动」。
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setElapsed((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const total = progress?.total ?? 0;
  const done = progress?.done ?? 0;
  const ratio = total > 0 ? done / total : 0;
  // 拿到总数之前（列出文件 + 探测第一批）进度柱用不确定态动画。
  const running = total > 0 && done < total;

  return (
    <div
      className="animate-fade fixed inset-0 z-[60] grid cursor-progress place-items-center bg-app/70 backdrop-blur-[3px]"
      role="alertdialog"
      aria-busy="true"
      aria-live="polite"
      aria-label={title}
    >
      <div className="animate-rise w-[380px] rounded-2xl border border-line bg-surface p-5 text-center shadow-[var(--shadow-pop)]">
        <span className="mx-auto grid h-12 w-12 place-items-center rounded-xl bg-accent-soft ring-1 ring-accent/25">
          <Spinner size={20} className="text-accent" />
        </span>
        <div className="mt-3 text-[13.5px] font-semibold text-ink">
          {title}
          {count && count > 0 ? `（${count} 项）` : ""}
        </div>

        {total > 0 ? (
          <div className="mt-3">
            <Progress
              value={ratio}
              running={running}
              label="正在读取元数据（ffprobe）"
              detail={`${done} / ${total}`}
            />
          </div>
        ) : (
          // 还没拿到总数（列目录 + 探测第一批）：用不确定进度条，避免看起来“卡住了”。
          <div className="mt-3">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-3 ring-1 ring-inset ring-line">
              <div className="progress-indeterminate h-full w-full rounded-full" />
            </div>
            <div className="mt-1.5 flex items-baseline gap-2 text-[11px]">
              <span className="text-ink-2">正在列出文件…</span>
              <span className="tnum ml-auto shrink-0 font-mono text-ink-3">
                已等待 {elapsed} 秒
              </span>
            </div>
          </div>
        )}

        <p className="mt-2.5 text-[11.5px] leading-relaxed text-ink-3">
          {hint ?? "文件越多耗时越久，请稍候…"}
        </p>
        <p className="tnum mt-1 font-mono text-[10.5px] text-ink-3">
          界面已锁定，扫描完成前请勿重复拖入（已等待 {elapsed} 秒）
        </p>
      </div>
    </div>
  );
}
