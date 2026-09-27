// 转换页（设计方案 §6.2）。

import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import type { ChannelMode, EncodeSpec, JobRequest, SampleRateOption } from "../types";
import { Collapsible, Field, NumberInput, Panel, Segmented, Select, Spinner, Switch } from "../components/ui";
import { Icon } from "../components/Icons";

export function ConvertPage() {
  const { formats, effectiveIds, outputDir, policy, itemsById, notify, registerRequestBuilder } = useApp();
  const [format, setFormat] = useState("flac");
  const [spec, setSpec] = useState<EncodeSpec | null>(null);
  const [keepTags, setKeepTags] = useState(true);
  const [gainDb, setGainDb] = useState<number | null>(null);
  const [ceilingDbtp, setCeilingDbtp] = useState<number | null>(null);
  const [channelMode, setChannelMode] = useState<ChannelMode>("keep");
  const [downmix, setDownmix] = useState(true);
  const [loading, setLoading] = useState(false);
  const [sampleRates, setSampleRates] = useState<SampleRateOption[]>([]);

  const selectedFormat = formats.find((option) => option.format === format);
  const firstId = effectiveIds[0];
  const firstItem = firstId ? itemsById.get(firstId) : undefined;

  // 首次拿到格式列表时选中第一个可用格式
  useEffect(() => {
    if (formats.length === 0) return;
    if (!formats.some((option) => option.format === format && option.available)) {
      const fallback = formats.find((option) => option.available);
      if (fallback) setFormat(fallback.format);
    }
  }, [formats, format]);

  const loadSpec = useCallback(
    async (target: string, itemId?: string) => {
      setLoading(true);
      try {
        // 参数与采样率选项一起拿：选项里已经按目标格式算好了推荐值
        const [nextSpec, nextRates] = await Promise.all([
          api.defaultEncodeSpec(target, itemId),
          api.sampleRateOptions(target, itemId),
        ]);
        const recommended = nextRates.find((option) => option.recommended)?.value ?? null;
        setSampleRates(nextRates);
        setSpec({ ...nextSpec, sampleRate: recommended });
      } catch (error) {
        notify("error", error instanceof Error ? error.message : String(error));
      } finally {
        setLoading(false);
      }
    },
    [notify],
  );

  useEffect(() => {
    if (!format) return;
    if (formats.length > 0 && !formats.some((option) => option.format === format)) return;
    void loadSpec(format, firstId);
  }, [format, firstId, formats, loadSpec]);

  useEffect(() => {
    registerRequestBuilder((): JobRequest | null => {
      if (!spec) return null;
      return {
        page: "convert",
        outputDir,
        policy,
        ids: effectiveIds,
        config: {
          spec: { ...spec, format },
          keepTags,
          gainDb,
          ceilingDbtp,
          channelMode,
          downmixFakeMultichannel: downmix,
        },
      };
    });
  });

  return (
    <div className="flex flex-col gap-3">
      <Panel title="输出格式" subtitle={selectedFormat?.reason ?? "由后端按格式推荐参数"} icon="convert">
        <div className="space-y-3">
          <Field label="格式">
            <Select
              value={format}
              onChange={setFormat}
              options={formats.map((option) => ({
                value: option.format,
                label: `${option.label}　.${option.extension}${option.lossless ? "　无损" : "　有损"}`,
                disabled: !option.available,
              }))}
            />
          </Field>

          <Field
            label="采样率"
            hint={
              sampleRates.find((option) => option.recommended)?.label
                ? `推荐：${sampleRates.find((option) => option.recommended)?.label}`
                : "按目标格式自动推荐"
            }
          >
            <Select
              value={spec?.sampleRate === null || spec?.sampleRate === undefined ? "auto" : String(spec.sampleRate)}
              onChange={(value) =>
                setSpec((prev) => (prev ? { ...prev, sampleRate: value === "auto" ? null : Number(value) } : prev))
              }
              options={sampleRates.map((option) => ({
                value: option.value === null ? "auto" : String(option.value),
                label: option.label,
              }))}
            />
          </Field>
          {sampleRates.length > 0 && !sampleRates[0].supported && (
            <div className="flex items-start gap-1.5 rounded-lg border border-amber-ink/25 bg-amber-soft px-2.5 py-2 text-[11px] text-ink-2">
              <Icon name="alert" size={12} className="mt-[1px] shrink-0 text-amber-ink" />
              当前目标格式不支持源采样率（{firstItem?.media?.sampleRate ?? "?"} Hz），已自动选好推荐的输出采样率。
            </div>
          )}
          {loading && (
            <div className="flex items-center gap-1.5 text-[11px] text-ink-3">
              <Spinner size={12} />
              正在读取推荐参数…
            </div>
          )}
          <Switch checked={keepTags} onChange={setKeepTags} label="保留元数据" hint="标签与封面" />
        </div>
      </Panel>

      <Collapsible title="高级选项" hint="位深 / 声道 / 增益" icon="sliders">
        <div className="grid grid-cols-2 gap-3">
          <Field label="位深">
            <Segmented
              value={spec?.bitDepth === null || spec?.bitDepth === undefined ? "auto" : String(spec.bitDepth)}
              onChange={(value) =>
                setSpec((prev) => (prev ? { ...prev, bitDepth: value === "auto" ? null : Number(value) } : prev))
              }
              options={[
                { value: "auto", label: "保持源" },
                { value: "16", label: "16" },
                { value: "24", label: "24" },
              ]}
              size="sm"
              className="w-full"
            />
          </Field>
          <Field label="声道">
            <Segmented
              value={spec?.channels === null || spec?.channels === undefined ? "auto" : String(spec.channels)}
              onChange={(value) =>
                setSpec((prev) => (prev ? { ...prev, channels: value === "auto" ? null : Number(value) } : prev))
              }
              options={[
                { value: "auto", label: "保持源" },
                { value: "1", label: "单声道" },
                { value: "2", label: "立体声" },
              ]}
              size="sm"
              className="w-full"
            />
          </Field>
        </div>
        <Field label="声道处理">
          <Segmented
            value={channelMode}
            onChange={setChannelMode}
            options={[
              { value: "keep", label: "保持源" },
              { value: "stereo", label: "立体声" },
              { value: "mono", label: "单声道" },
            ]}
            size="sm"
            className="w-full"
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="增益" hint="留空 = 不处理">
            <NumberInput value={gainDb} step={0.1} min={-30} max={30} placeholder="0.0" suffix="dB" onChange={setGainDb} />
          </Field>
          <Field label="真峰值天花板" hint="配合增益">
            <NumberInput
              value={ceilingDbtp}
              step={0.1}
              min={-6}
              max={0}
              placeholder="不限幅"
              suffix="dBTP"
              onChange={setCeilingDbtp}
            />
          </Field>
        </div>
        <Switch checked={downmix} onChange={setDownmix} label="自动折混假多声道" hint="后声道为静音时折混为立体声" />
        {effectiveIds.length > 1 && <p className="text-[10.5px] text-ink-3">多选时以列表中第一个文件为参考。</p>}
      </Collapsible>
    </div>
  );
}
