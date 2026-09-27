// 音量归一化页（设计方案 §6.4、§7.3–§7.5）。

import { useEffect, useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import type { GainPreviewRow, JobRequest, NormalizeParams } from "../types";
import {
  Badge,
  Button,
  Field,
  NumberInput,
  Collapsible,
  Panel,
  Progress,
  Segmented,
  Switch,
} from "../components/ui";
import { Icon } from "../components/Icons";

const FALLBACK: NormalizeParams = {
  targetLufs: -14,
  ceilingDbtp: -1,
  lookaheadMs: 3,
  attackMs: 5,
  releaseMs: 120,
  downmixFakeMultichannel: true,
};

const TARGET_PRESETS: { label: string; lufs: number; hint: string }[] = [
  { label: "流媒体", lufs: -14, hint: "Spotify / Apple Music" },
  { label: "CD", lufs: -9, hint: "传统唱片" },
  { label: "播客", lufs: -16, hint: "人声优先" },
  { label: "广播", lufs: -23, hint: "EBU R128" },
];

export function NormalizePage() {
  const {
    normalizeDefaults,
    formats,
    effectiveIds,
    outputDir,
    policy,
    itemsById,
    measure,
    measureProgress,
    measurements,
    notify,
    registerRequestBuilder,
  } = useApp();
  const [params, setParams] = useState<NormalizeParams>(FALLBACK);
  const [format, setFormat] = useState("flac");
  const [bitDepth, setBitDepth] = useState<number | null>(24);
  const [keepTags, setKeepTags] = useState(true);
  const [gainRows, setGainRows] = useState<GainPreviewRow[]>([]);

  useEffect(() => {
    if (normalizeDefaults) setParams(normalizeDefaults);
  }, [normalizeDefaults]);

  useEffect(() => {
    registerRequestBuilder((): JobRequest | null => ({
      page: "normalize",
      outputDir,
      policy,
      ids: effectiveIds,
      params,
      format,
      bitDepth,
      keepTags,
    }));
  });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await api.gainPreview(effectiveIds, params);
        if (!cancelled) setGainRows(rows);
      } catch {
        if (!cancelled) setGainRows([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [effectiveIds, params, measurements]);

  const measuredCount = gainRows.filter((row) => !row.needsMeasuring).length;
  const pendingCount = gainRows.length - measuredCount;
  const hasFake = gainRows.length > 0 && effectiveIds.some((id) => itemsById.get(id)?.fakeMultichannel);

  return (
    <div className="flex flex-col gap-3">
      <Panel
        title="响度目标"
        icon="gauge"
        actions={
          <>
            <Badge tone={pendingCount === 0 && gainRows.length > 0 ? "ok" : "warn"}>
              已测 {measuredCount}/{gainRows.length}
            </Badge>
            {measureProgress && (
              <Badge tone="accent">
                测量中 {measureProgress.done}/{measureProgress.total}
              </Badge>
            )}
            <Button
              size="sm"
              icon="gauge"
              loading={measureProgress !== null}
              disabled={effectiveIds.length === 0 || measureProgress !== null}
              onClick={() => void measure(effectiveIds)}
            >
              测量
            </Button>
          </>
        }
      >
        {measureProgress && (
          <div className="mb-3">
            <Progress
              value={measureProgress.done / Math.max(1, measureProgress.total)}
              label="正在测量响度与真峰值…"
              detail={`${measureProgress.done} / ${measureProgress.total} 个文件`}
            />
          </div>
        )}
        <div className="mb-3 flex flex-wrap gap-1.5">
          {TARGET_PRESETS.map((preset) => {
            const active = Math.abs(params.targetLufs - preset.lufs) < 0.05;
            return (
              <button
                key={preset.label}
                type="button"
                title={preset.hint}
                onClick={() => setParams((prev) => ({ ...prev, targetLufs: preset.lufs }))}
                className={`rounded-md px-2 py-1 text-[11px] transition-colors ${
                  active
                    ? "bg-accent/15 text-accent ring-1 ring-accent/30"
                    : "bg-surface-2 text-ink-2 ring-1 ring-inset ring-line hover:text-ink"
                }`}
              >
                {preset.label} <span className="tnum font-mono opacity-70">{preset.lufs}</span>
              </button>
            );
          })}
        </div>

        <Field label="目标集成响度">
            <NumberInput
              value={params.targetLufs}
              step={0.5}
              min={-30}
              max={0}
              suffix="LUFS"
              onChange={(value) => setParams((prev) => ({ ...prev, targetLufs: value ?? -14 }))}
            />
        </Field>

        <Collapsible title="高级选项" hint="真峰值 / 前瞻 / 折混" icon="sliders">
          <Field label="真峰值上限" hint="留空 = 不限幅">
            <NumberInput
              value={params.ceilingDbtp}
              step={0.1}
              min={-6}
              max={0}
              placeholder="不限幅"
              suffix="dBTP"
              onChange={(value) => setParams((prev) => ({ ...prev, ceilingDbtp: value }))}
            />
          </Field>
      <div className="mt-3 grid grid-cols-3 gap-3">
        <Field label="前视">
          <NumberInput
            value={params.lookaheadMs}
            step={0.5}
            min={0}
            max={20}
            suffix="ms"
            onChange={(value) => setParams((prev) => ({ ...prev, lookaheadMs: value ?? 3 }))}
          />
        </Field>
        <Field label="攻击">
          <NumberInput
            value={params.attackMs}
            step={0.5}
            min={0}
            max={100}
            suffix="ms"
            onChange={(value) => setParams((prev) => ({ ...prev, attackMs: value ?? 5 }))}
          />
        </Field>
        <Field label="释放">
          <NumberInput
            value={params.releaseMs}
            step={5}
            min={1}
            max={2000}
            suffix="ms"
            onChange={(value) => setParams((prev) => ({ ...prev, releaseMs: value ?? 120 }))}
          />
        </Field>
      </div>

      <div className="mt-3.5">
        <Switch
          checked={params.downmixFakeMultichannel}
          onChange={(value) => setParams((prev) => ({ ...prev, downmixFakeMultichannel: value }))}
          label="把假多声道折混为立体声"
          hint="后声道全为静音时，避免输出无意义的声道数"
        />
        {hasFake && (
          <div className="mt-2 flex items-start gap-1.5 rounded-lg border border-amber-ink/25 bg-amber-soft/[0.06] px-2.5 py-2 text-[11px] text-ink-2">
            <Icon name="alert" size={12} className="mt-[1px] shrink-0" />
            选中文件里检测到假多声道，建议保持开启。
          </div>
        )}
      </div>
        </Collapsible>
      </Panel>

      <Panel title="输出编码" subtitle="归一化必须重新编码，建议使用无损格式" icon="convert">
        <div className="space-y-3">
          <Field label="格式">
            <Segmented
              value={formats.some((option) => option.format === format && option.available) ? format : formats.find((o) => o.available)?.format ?? format}
              onChange={setFormat}
              options={formats
                .filter((option) => option.lossless || option.available)
                .slice(0, 5)
                .map((option) => ({ value: option.format, label: option.label }))}
              size="sm"
              className="w-full"
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="位深">
              <Segmented
                value={bitDepth === null ? "auto" : String(bitDepth)}
                onChange={(value) => setBitDepth(value === "auto" ? null : Number(value))}
                options={[
                  { value: "auto", label: "保持源" },
                  { value: "16", label: "16" },
                  { value: "24", label: "24" },
                ]}
                size="sm"
                className="w-full"
              />
            </Field>
          </div>
          <Switch checked={keepTags} onChange={setKeepTags} label="保留元数据" hint="标签与封面" />
        </div>
      </Panel>

      <Panel title="增益预估" subtitle={`${gainRows.length} 项`} icon="waveform">
        <div className="max-h-[220px] overflow-auto rounded-xl border border-line">
          <table className="w-full border-collapse">
            <thead className="sticky top-0 z-10 bg-surface-2/95 backdrop-blur">
              <tr className="text-left text-[10.5px] uppercase tracking-wider text-ink-3">
                <th className="px-2.5 py-2">文件</th>
                <th className="w-20 px-2.5 py-2 text-right">测得</th>
                <th className="w-24 px-2.5 py-2 text-right">真峰值</th>
                <th className="w-20 px-2.5 py-2 text-right">增益</th>
                <th className="w-20 px-2.5 py-2">状态</th>
              </tr>
            </thead>
            <tbody>
              {gainRows.length === 0 ? (
                <tr>
                  <td colSpan={5} className="px-2.5 py-6 text-center text-[12px] text-ink-3">
                    请先选择文件
                  </td>
                </tr>
              ) : (
                gainRows.map((row) => (
                  <tr key={row.id} className="border-t border-line">
                    <td className="max-w-[150px] truncate px-2.5 py-1.5 text-[11.5px] text-ink-2" title={row.fileName}>
                      {row.fileName}
                    </td>
                    <td className="tnum px-2.5 py-1.5 text-right font-mono text-[11px] text-ink-2">
                      {row.measuredLufs === null ? "—" : `${row.measuredLufs.toFixed(1)}`}
                    </td>
                    <td className="tnum px-2.5 py-1.5 text-right font-mono text-[11px] text-ink-3">
                      {row.truePeakDbtp === null ? "—" : row.truePeakDbtp.toFixed(1)}
                    </td>
                    <td className="tnum px-2.5 py-1.5 text-right font-mono text-[11px] text-accent">
                      {row.needsMeasuring ? "—" : `${row.gainDb >= 0 ? "+" : ""}${row.gainDb.toFixed(1)}`}
                    </td>
                    <td className="px-2.5 py-1.5">
                      {row.needsMeasuring ? <Badge tone="warn">待测量</Badge> : <Badge tone="ok">就绪</Badge>}
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
