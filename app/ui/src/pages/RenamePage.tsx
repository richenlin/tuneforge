// 文件名标准化页（设计方案 §6.3、§9）。

import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import type { JobRequest, MissingFieldPolicy, RenamePreviewItem, SanitizeOptions } from "../types";
import {
  Badge,
  Collapsible,
  Field,
  NumberInput,
  Panel,
  Segmented,
  Switch,
  TextInput,
} from "../components/ui";
import { Icon } from "../components/Icons";

const DEFAULT_SANITIZE: SanitizeOptions = {
  replacement: "_",
  collapseSpaces: true,
  fullwidthToHalfwidth: true,
  maxLen: 120,
  fallback: "未命名",
};

const VARIABLES = ["{title}", "{artist}", "{album}", "{albumArtist}", "{track}", "{year}", "{ext}"];

export function RenamePage() {
  const { presets, effectiveIds, outputDir, policy, notify, registerRequestBuilder } = useApp();
  const [template, setTemplate] = useState("{artist} - {title}");
  const [missing, setMissing] = useState<MissingFieldPolicy>("empty");
  const [sanitize, setSanitize] = useState<SanitizeOptions>(DEFAULT_SANITIZE);
  const [preview, setPreview] = useState<RenamePreviewItem[]>([]);

  useEffect(() => {
    registerRequestBuilder((): JobRequest | null => ({
      page: "rename",
      outputDir,
      policy,
      template,
      missing,
      sanitize,
      ids: effectiveIds,
    }));
  });

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const rows = await api.previewRename({ template, missing, sanitize, outputDir, ids: effectiveIds });
          if (!cancelled) setPreview(rows);
        } catch (error) {
          if (!cancelled) notify("error", error instanceof Error ? error.message : String(error));
        }
      })();
    }, 200);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [template, missing, sanitize, outputDir, effectiveIds, notify]);

  const conflicts = useMemo(() => preview.filter((row) => row.conflict !== null).length, [preview]);
  const changed = useMemo(() => preview.filter((row) => row.changed && !row.skip).length, [preview]);

  return (
    <div className="flex flex-col gap-3">
      <Panel title="命名模板" subtitle="实时预览，不落盘" icon="pencil">
        <Field label="模板">
          <TextInput value={template} onChange={setTemplate} mono placeholder="{artist} - {title}" />
        </Field>

        <div className="mt-2 flex flex-wrap gap-1.5">
          {presets.map((preset) => (
            <button
              key={preset.id}
              type="button"
              onClick={() => setTemplate(preset.template)}
              title={preset.template}
              className={`rounded-md px-2 py-1 text-[11px] transition-colors ${
                template === preset.template
                  ? "bg-accent/15 text-accent ring-1 ring-accent/30"
                  : "bg-surface-2 text-ink-2 ring-1 ring-inset ring-line hover:text-ink"
              }`}
            >
              {preset.label}
            </button>
          ))}
        </div>

        <div className="mt-3 flex flex-wrap gap-1.5">
          {VARIABLES.map((variable) => (
            <button
              key={variable}
              type="button"
              onClick={() => setTemplate((prev) => `${prev}${prev.endsWith(" ") || prev === "" ? "" : " "}${variable}`)}
              className="rounded bg-surface-3 px-1.5 py-[3px] font-mono text-[10.5px] text-ink-3 ring-1 ring-inset ring-line hover:text-accent"
            >
              {variable}
            </button>
          ))}
        </div>

        <div className="mt-3.5">
          <Field label="字段缺失时">
            <Segmented
              value={missing}
              onChange={setMissing}
              options={[
                { value: "empty", label: "留空" },
                { value: "placeholder", label: "保留占位符" },
                { value: "skip", label: "跳过" },
              ]}
              size="sm"
              className="w-full"
            />
          </Field>
        </div>

        <Collapsible title="高级选项" hint="非法字符 / 长度 / 回退名" icon="sliders">
  <div className="grid grid-cols-2 gap-3">
    <Field label="非法字符替换为">
      <TextInput
        value={sanitize.replacement}
        onChange={(value) => setSanitize((prev) => ({ ...prev, replacement: value.slice(-1) || "_" }))}
      />
    </Field>
    <Field label="最大长度" hint="不含扩展名">
      <NumberInput
        value={sanitize.maxLen}
        min={16}
        max={200}
        suffix="字符"
        onChange={(value) => setSanitize((prev) => ({ ...prev, maxLen: value ?? 120 }))}
      />
    </Field>
  </div>

  <Field label="全部字段缺失时的回退名">
    <TextInput value={sanitize.fallback} onChange={(value) => setSanitize((prev) => ({ ...prev, fallback: value }))} />
  </Field>

  <div className="space-y-2.5">
    <Switch
      checked={sanitize.collapseSpaces}
      onChange={(value) => setSanitize((prev) => ({ ...prev, collapseSpaces: value }))}
      label="折叠连续空格"
    />
    <Switch
      checked={sanitize.fullwidthToHalfwidth}
      onChange={(value) => setSanitize((prev) => ({ ...prev, fullwidthToHalfwidth: value }))}
      label="全角转半角"
    />
  </div>
        </Collapsible>
      </Panel>

      <Panel
        title="重命名预览"
        subtitle={`${preview.length} 项 · ${changed} 项将改名`}
        icon="list"
        actions={
          <>
            {conflicts > 0 && <Badge tone="err" icon="alert">{conflicts} 项冲突</Badge>}
            {effectiveIds.length === 0 && <Badge tone="warn">未选择文件</Badge>}
          </>
        }
      >
        <div className="max-h-[300px] overflow-auto rounded-xl border border-line">
          <table className="w-full border-collapse">
            <thead className="sticky top-0 z-10 bg-surface-2/95 backdrop-blur">
              <tr className="text-left text-[10.5px] uppercase tracking-wider text-ink-3">
                <th className="px-2.5 py-2">原名</th>
                <th className="px-2.5 py-2">新名</th>
                <th className="w-20 px-2.5 py-2">状态</th>
              </tr>
            </thead>
            <tbody>
              {preview.length === 0 ? (
                <tr>
                  <td colSpan={3} className="px-2.5 py-6 text-center text-[12px] text-ink-3">
                    没有可预览的文件
                  </td>
                </tr>
              ) : (
                preview.map((row) => (
                  <tr
                    key={row.sourcePath}
                    className={`border-t border-line ${
                      row.conflict ? "bg-rose-soft" : row.changed && !row.skip ? "bg-mint-soft" : ""
                    }`}
                  >
                    <td className="max-w-[150px] truncate px-2.5 py-1.5 text-[11.5px] text-ink-3" title={row.sourcePath}>
                      {row.originalName}
                    </td>
                    <td className="max-w-[170px] px-2.5 py-1.5">
                      <div className="truncate font-mono text-[11.5px] text-ink" title={row.newName}>
                        {row.newName}
                      </div>
                      {row.notes.length > 0 && (
                        <div className="truncate text-[10.5px] text-ink-3">{row.notes.join("；")}</div>
                      )}
                    </td>
                    <td className="px-2.5 py-1.5">
                      {row.skip ? (
                        <Badge tone="warn">跳过</Badge>
                      ) : row.conflict ? (
                        <Badge tone="err">冲突</Badge>
                      ) : row.changed ? (
                        <Badge tone="ok" icon="check">
                          改名
                        </Badge>
                      ) : (
                        <span className="text-[11px] text-ink-3">不变</span>
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Panel>
    </div>
  );
}
