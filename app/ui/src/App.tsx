// 主布局：品牌顶栏（导航 / 主题 / ffmpeg 状态）+ 输入工具条 + 曲目列表与参数面板 + 底部控制台。

import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { useApp } from "./store";
import type { AppInfo, PageId } from "./types";
import { applyTheme, getStoredTheme, type Theme } from "./theme";
import { FfmpegBanner } from "./components/FfmpegBanner";
import { FileList } from "./components/FileList";
import { JobBar } from "./components/JobBar";
import { Toolbar } from "./components/Toolbar";
import { Badge, Button, Divider, IconButton, Spinner } from "./components/ui";
import { Icon, Logo, type IconName } from "./components/Icons";
import { ConvertPage } from "./pages/ConvertPage";
import { RenamePage } from "./pages/RenamePage";
import { NormalizePage } from "./pages/NormalizePage";
import { TagsPage } from "./pages/TagsPage";

const PAGES: { id: PageId; label: string; startLabel: string; hint: string; icon: IconName }[] = [
  { id: "convert", label: "格式转换", startLabel: "开始转换", hint: "解码 → DSP → 重新编码", icon: "convert" },
  { id: "rename", label: "文件名标准化", startLabel: "开始重命名", hint: "只改文件名，不重新编码", icon: "pencil" },
  { id: "normalize", label: "音量归一化", startLabel: "开始归一化", hint: "EBU R128 + 真峰值限幅", icon: "sliders" },
  { id: "tags", label: "标签元素", startLabel: "写入标签", hint: "只改元数据，不重新编码", icon: "tag" },
];

function FfmpegStatusChip() {
  const { ffmpeg, ffmpegReady, ffmpegProbing, refreshFfmpeg, pickFfmpegDir } = useApp();
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="relative" ref={boxRef}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={
          ffmpegProbing
            ? "正在后台检测 FFmpeg（不阻塞界面）"
            : ffmpegReady
              ? `${ffmpeg?.source ?? ""}${ffmpeg?.probeMs !== null && ffmpeg?.probeMs !== undefined ? ` · 探测 ${ffmpeg.probeMs} ms` : ""}`
              : (ffmpeg?.message ?? "FFmpeg 不可用")
        }
        className={`inline-flex h-[34px] items-center gap-2 rounded-lg border px-2.5 text-[12px] font-medium transition-colors ${
          ffmpegReady
            ? "border-mint/25 bg-mint-soft text-mint hover:brightness-[0.98]"
            : ffmpegProbing
              ? "border-line bg-surface-2 text-ink-2 hover:bg-surface-3"
              : "border-rose-ink/25 bg-rose-soft text-rose-ink"
        }`}
      >
        {ffmpegProbing ? (
          <Spinner size={12} className="text-accent" />
        ) : (
          <span className={`h-1.5 w-1.5 rounded-full ${ffmpegReady ? "bg-mint" : "bg-rose-ink"}`} />
        )}
        {ffmpegReady ? "FFmpeg 就绪" : ffmpegProbing ? "FFmpeg 检测中…" : "FFmpeg 未就绪"}
        <Icon name="chevronDown" size={13} className="opacity-60" />
      </button>

      {open && (
        <div className="animate-rise absolute right-0 top-[42px] z-40 w-[392px] rounded-xl border border-line bg-surface p-3 shadow-[var(--shadow-pop)]">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[12px] font-semibold text-ink">FFmpeg 运行时</span>
            <div className="flex items-center gap-1.5">
              {ffmpeg?.source && <Badge tone="accent">{ffmpeg.source}</Badge>}
              {ffmpegReady && ffmpeg?.probeMs !== null && ffmpeg?.probeMs !== undefined && (
                <Badge icon="clock">
                  {ffmpeg.probeMs} ms{ffmpeg.cached ? " · 缓存" : ""}
                </Badge>
              )}
            </div>
          </div>
          <div className="space-y-1.5 text-[11.5px]">
            {ffmpegProbing ? (
              <p className="flex items-center gap-2 text-ink-2">
                <Spinner size={13} className="text-accent" />
                正在后台探测（首次启动需要加载 ffmpeg，界面不受影响）…
              </p>
            ) : (
              <>
                <div className="truncate font-mono text-ink-2" title={ffmpeg?.version ?? ""}>
                  {ffmpeg?.version ?? "未检测到"}
                </div>
                <div className="truncate font-mono text-ink-3" title={ffmpeg?.ffmpegPath ?? ""}>
                  {ffmpeg?.ffmpegPath ?? "—"}
                </div>
              </>
            )}
            {ffmpegReady && (
              <div className="flex flex-wrap gap-1 pt-1">
                {(ffmpeg?.encodable ?? []).map((format) => (
                  <Badge key={format}>{format}</Badge>
                ))}
              </div>
            )}
            {!ffmpegReady && !ffmpegProbing && ffmpeg?.message && (
              <p className="text-rose-ink/90">{ffmpeg.message}</p>
            )}
          </div>
          <div className="mt-3 flex items-center gap-2">
            <Button size="sm" variant="primary" icon="folder" onClick={() => void pickFfmpegDir()}>
              指定 ffmpeg 目录
            </Button>
            <Button size="sm" icon="refresh" onClick={() => void refreshFfmpeg()}>
              重新检测
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

export function App() {
  const { page, setPage, buildRequest, items, toast, dismissToast } = useApp();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [theme, setTheme] = useState<Theme>(() => getStoredTheme());

  useEffect(() => {
    void (async () => {
      try {
        setInfo(await api.appInfo());
      } catch {
        setInfo(null);
      }
    })();
  }, []);

  const toggleTheme = () => {
    const next: Theme = theme === "light" ? "dark" : "light";
    setTheme(next);
    applyTheme(next);
  };

  const current = PAGES.find((entry) => entry.id === page) ?? PAGES[0];

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-14 shrink-0 items-center gap-4 border-b border-line bg-surface/80 px-4 backdrop-blur-md">
        <div className="flex items-center gap-2.5">
          <Logo size={30} />
          <div className="leading-tight">
            <div className="flex items-baseline gap-1.5">
              <span className="text-[15px] font-semibold tracking-tight text-ink">Tuneforge</span>
              <span className="tnum font-mono text-[10.5px] text-ink-3">v{info?.version ?? "0.1.0"}</span>
            </div>
          </div>
        </div>

        <Divider vertical />

        <nav className="inline-flex rounded-xl border border-line bg-surface-2 p-[3px]">
          {PAGES.map((entry) => {
            const active = page === entry.id;
            return (
              <button
                key={entry.id}
                type="button"
                title={entry.hint}
                onClick={() => setPage(entry.id)}
                className={`inline-flex h-8 items-center gap-2 rounded-[9px] px-3 text-[12.5px] font-medium transition-all duration-150 ${
                  active
                    ? "bg-surface text-ink shadow-[0_1px_2px_rgba(15,23,42,0.12)] ring-1 ring-accent/35"
                    : "text-ink-3 hover:bg-surface-3 hover:text-ink"
                }`}
              >
                <Icon name={entry.icon} size={15} className={active ? "text-accent" : "text-ink-3"} />
                {entry.label}
              </button>
            );
          })}
        </nav>

        <div className="ml-auto flex items-center gap-2">
          <FfmpegStatusChip />
          <IconButton
            icon={theme === "light" ? "moon" : "sun"}
            title={theme === "light" ? "切换到深色主题" : "切换到浅色主题"}
            onClick={toggleTheme}
          />
        </div>
      </header>

      <FfmpegBanner />
      <Toolbar />

      <main className="flex min-h-0 flex-1 gap-3 p-3">
        <div className="flex min-w-0 flex-1 flex-col">
          <FileList />
        </div>
        <aside className="flex w-[392px] shrink-0 flex-col overflow-y-auto pr-0.5">
          <div className="mb-2 flex items-center gap-2 px-0.5">
            <Icon name={current.icon} size={15} className="text-accent" />
            <h2 className="text-[13px] font-semibold text-ink">{current.label}</h2>
          </div>
          <div className="flex flex-col gap-3 pb-1">
            {page === "convert" && <ConvertPage />}
            {page === "rename" && <RenamePage />}
            {page === "normalize" && <NormalizePage />}
            {page === "tags" && <TagsPage />}
          </div>
        </aside>
      </main>

      <JobBar buildRequest={buildRequest} startLabel={current.startLabel} />

      <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-line bg-surface-2 px-4 text-[10.5px] text-ink-3">
        <Icon name="info" size={12} />
        <span className="truncate">{info?.notes?.join("　·　") ?? "只读源文件，结果输出到新文件夹"}</span>
      </footer>

      {toast && (
        <div className="pointer-events-none fixed bottom-24 left-1/2 z-50 -translate-x-1/2">
          <button
            type="button"
            onClick={dismissToast}
            className={`animate-rise pointer-events-auto inline-flex max-w-[560px] items-center gap-2.5 rounded-xl border bg-surface px-3.5 py-2.5 text-[12.5px] text-ink shadow-[var(--shadow-pop)] ${
              toast.kind === "error"
                ? "border-rose-ink/30"
                : toast.kind === "success"
                  ? "border-mint/30"
                  : "border-line"
            }`}
          >
            <Icon
              name={toast.kind === "error" ? "xCircle" : toast.kind === "success" ? "checkCircle" : "info"}
              size={15}
              className={toast.kind === "error" ? "text-rose-ink" : toast.kind === "success" ? "text-mint" : "text-accent"}
            />
            <span className="text-left">{toast.text}</span>
            <Icon name="x" size={12} className="text-ink-3" />
          </button>
        </div>
      )}
    </div>
  );
}
