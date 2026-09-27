// 曲目列表（中间主区域）：搜索 / 多选 / 技术参数与测量结果一览。

import { useMemo, useState } from "react";
import type { MediaItem } from "../types";
import { useApp } from "../store";
import {
  Badge,
  EmptyHint,
  RATING_LABEL,
  TextInput,
  formatBytes,
  formatDb,
  formatDuration,
} from "./ui";
import { Icon } from "./Icons";

function techSummary(item: MediaItem): string[] {
  const info = item.media;
  if (!info) return [];
  const parts: string[] = [];
  if (info.codec) parts.push(info.codec);
  if (info.sampleRate) parts.push(`${(info.sampleRate / 1000).toFixed(info.sampleRate % 1000 === 0 ? 0 : 1)} kHz`);
  if (info.bitsPerSample) parts.push(`${info.bitsPerSample} bit${info.isFloat ? "f" : ""}`);
  if (info.channels) parts.push(`${info.channels} ch`);
  return parts;
}

function displayTitle(item: MediaItem): string {
  const title = item.tags.title?.trim();
  const artist = item.tags.artist?.trim();
  if (title && artist) return `${artist} — ${title}`;
  return title ?? "";
}

function matches(item: MediaItem, query: string): boolean {
  if (!query) return true;
  const haystack = [
    item.fileName,
    item.path,
    item.tags.title,
    item.tags.artist,
    item.tags.album,
    item.tags.genre,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(query.toLowerCase());
}

export function FileList() {
  const { items, selected, toggleSelect, setSelection, measurements } = useApp();
  const [query, setQuery] = useState("");

  const visible = useMemo(() => items.filter((item) => matches(item, query)), [items, query]);
  const allVisibleSelected = visible.length > 0 && visible.every((item) => selected.includes(item.id));

  return (
    <section className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-[inset_0_1px_0_0_rgba(255,255,255,0.03)] backdrop-blur-sm">
      <header className="flex shrink-0 items-center gap-3 border-b border-line px-3.5 py-2.5">
        <div className="flex items-center gap-2.5">
          <span className="grid h-7 w-7 place-items-center rounded-lg bg-surface-2 text-accent ring-1 ring-line/70">
            <Icon name="music" size={15} />
          </span>
          <h2 className="text-[13px] font-semibold tracking-wide text-ink">曲目列表</h2>
          <Badge>{visible.length === items.length ? items.length : `${visible.length} / ${items.length}`}</Badge>
          {selected.length > 0 && <Badge tone="accent">已选 {selected.length}</Badge>}
        </div>
        <div className="ml-auto w-[240px]">
          <TextInput value={query} onChange={setQuery} placeholder="搜索文件名 / 标签…" prefixIcon="search" />
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-auto">
        {items.length === 0 ? (
          <div className="p-4">
            <EmptyHint>
              还没有曲目。点击上方「添加文件夹 / 添加文件」，或把文件直接拖进窗口即可开始扫描。
            </EmptyHint>
          </div>
        ) : (
          <table className="w-full border-collapse">
            <thead className="sticky top-0 z-10 bg-surface-2/95 backdrop-blur">
              <tr className="text-left text-[10.5px] uppercase tracking-wider text-ink-3">
                <th className="w-9 border-l-2 border-l-transparent px-2 py-2">
                  <input
                    type="checkbox"
                    aria-label="全选"
                    className="h-3.5 w-3.5"
                    checked={allVisibleSelected}
                    onChange={() =>
                      setSelection(
                        allVisibleSelected
                          ? selected.filter((id) => !visible.some((item) => item.id === id))
                          : Array.from(new Set([...selected, ...visible.map((item) => item.id)])),
                      )
                    }
                  />
                </th>
                <th className="w-8 px-1 py-2 text-right">#</th>
                <th className="px-2 py-2">文件 / 标题</th>
                <th className="w-[168px] px-2 py-2">技术参数</th>
                <th className="w-16 px-2 py-2 text-right">时长</th>
                <th className="w-20 px-2 py-2">评级</th>
                <th className="w-[150px] px-2 py-2 text-right">响度 / 真峰值</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((item, index) => {
                const isSelected = selected.includes(item.id);
                const rating = item.rating ? RATING_LABEL[item.rating] : null;
                const measured = measurements.get(item.id);
                const lufs = measured?.loudnessLufs ?? item.loudnessLufs;
                const peak = measured?.truePeakDbtp ?? item.truePeakDbtp;
                const tech = techSummary(item);
                const subtitle = displayTitle(item);
                return (
                  <tr
                    key={item.id}
                    onClick={() => toggleSelect(item.id)}
                    className={`group cursor-pointer border-t border-line transition-colors ${
                      isSelected ? "bg-accent/[0.07]" : "hover:bg-surface-3"
                    }`}
                  >
                    <td
                      className={`border-l-2 px-2 py-2 align-middle ${
                        isSelected ? "border-l-accent" : "border-l-transparent"
                      }`}
                    >
                      <input
                        type="checkbox"
                        aria-label={`选择 ${item.fileName}`}
                        className="h-3.5 w-3.5"
                        checked={isSelected}
                        onChange={() => toggleSelect(item.id)}
                      />
                    </td>
                    <td className="tnum px-1 py-2 text-right align-middle font-mono text-[11px] text-ink-3">
                      {index + 1}
                    </td>
                    <td className="max-w-[320px] px-2 py-2 align-middle">
                      <div
                        className={`truncate text-[12.5px] font-medium ${
                          item.error ? "text-rose-ink" : "text-ink"
                        }`}
                        title={item.path}
                      >
                        {item.fileName}
                      </div>
                      {(subtitle || item.hasCover) && (
                        <div className="flex items-center gap-1.5 truncate text-[11px] text-ink-3">
                          {subtitle && <span className="truncate">{subtitle}</span>}
                          {item.hasCover && (
                            <span className="inline-flex shrink-0 items-center gap-1 text-mint/80" title="含封面">
                              <Icon name="picture" size={11} />
                            </span>
                          )}
                        </div>
                      )}
                      {item.error && (
                        <div className="mt-0.5 flex items-center gap-1 truncate text-[11px] text-rose-ink/80" title={item.error}>
                          <Icon name="alert" size={11} />
                          {item.error}
                        </div>
                      )}
                    </td>
                    <td className="px-2 py-2 align-middle">
                      <div className="flex flex-wrap items-center gap-1">
                        {tech.slice(0, 2).map((part) => (
                          <span
                            key={part}
                            className="rounded bg-surface-2 px-1.5 py-[2px] font-mono text-[10.5px] text-ink-2 ring-1 ring-inset ring-line"
                          >
                            {part}
                          </span>
                        ))}
                        {tech.length > 2 && <span className="text-[10.5px] text-ink-3">{tech.slice(2).join(" · ")}</span>}
                        {item.media?.sizeBytes ? (
                          <span className="text-[10.5px] text-ink-3">{formatBytes(item.media.sizeBytes)}</span>
                        ) : null}
                        {item.fakeMultichannel && <Badge tone="warn">假多声道</Badge>}
                      </div>
                    </td>
                    <td className="tnum px-2 py-2 text-right align-middle font-mono text-[11.5px] text-ink-2">
                      {formatDuration(item.media?.durationSecs ?? null)}
                    </td>
                    <td className="px-2 py-2 align-middle">
                      {rating ? <Badge tone={rating.tone}>{rating.text}</Badge> : <span className="text-ink-3">—</span>}
                    </td>
                    <td className="tnum px-2 py-2 text-right align-middle font-mono text-[11.5px] text-ink-2">
                      {lufs === null || lufs === undefined ? (
                        <span className="text-ink-3">—</span>
                      ) : (
                        `${lufs.toFixed(1)} LUFS`
                      )}
                      <span className="ml-1.5 text-[10.5px] text-ink-3">
                        {peak === null || peak === undefined ? "" : formatDb(peak, " dBTP")}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}
