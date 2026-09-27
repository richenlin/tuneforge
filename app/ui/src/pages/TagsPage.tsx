// 标签元素页（设计方案 §6.5、§8）。

import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { useApp } from "../store";
import { TAG_FIELDS, type JobRequest, type TagField } from "../types";
import { Badge, Button, Collapsible, Field, Panel, Segmented, Switch, TextInput } from "../components/ui";
import { Icon } from "../components/Icons";

export function TagsPage() {
  const {
    effectiveIds,
    itemsById,
    outputDir,
    policy,
    refreshItems,
    notify,
    pickImageFile,
    registerRequestBuilder,
  } = useApp();
  const [values, setValues] = useState<Record<string, string>>({});
  const [enabled, setEnabled] = useState<Record<string, boolean>>({});
  const [find, setFind] = useState("");
  const [replaceWith, setReplaceWith] = useState("");
  const [findField, setFindField] = useState<TagField>("title");
  const [caseInsensitive, setCaseInsensitive] = useState(true);
  const [guessTemplate, setGuessTemplate] = useState("{artist} - {title}");
  const [cover, setCover] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [exportHint, setExportHint] = useState<string | null>(null);

  const firstId = effectiveIds[0];
  const single = effectiveIds.length === 1;

  /** 一键清除时保留的字段：艺术家与歌曲名。 */
  const KEEP_FIELDS: TagField[] = ["artist", "title"];

  useEffect(() => {
    registerRequestBuilder((): JobRequest | null => ({
      page: "tags",
      outputDir,
      policy,
      ids: effectiveIds,
    }));
  });

  useEffect(() => {
    if (!single || !firstId) return;
    const item = itemsById.get(firstId);
    if (!item) return;
    const next: Record<string, string> = {};
    for (const field of TAG_FIELDS) {
      const value = (item.tags as Record<string, unknown>)[field.id];
      next[field.id] = value === null || value === undefined ? "" : String(value);
    }
    setValues(next);
    setEnabled({});
  }, [single, firstId, itemsById]);

  useEffect(() => {
    if (!single || !firstId) {
      setCover(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const url = await api.coverPreview(firstId);
        if (!cancelled) setCover(url);
      } catch {
        if (!cancelled) setCover(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [single, firstId]);

  const runBusy = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
    } catch (error) {
      notify("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const enabledCount = TAG_FIELDS.filter((field) => enabled[field.id]).length;

  const applyEdits = () =>
    runBusy(async () => {
      const edits = TAG_FIELDS.filter((field) => enabled[field.id]).map((field) => ({
        field: field.id,
        value: values[field.id]?.trim() ? values[field.id] : null,
      }));
      if (edits.length === 0) {
        notify("info", "请先勾选要修改的字段");
        return;
      }
      await api.updateTags(effectiveIds, edits);
      await refreshItems();
      notify("success", `已更新 ${effectiveIds.length} 个文件的 ${edits.length} 个字段`);
    });

  /** 一键清除除「艺术家 / 标题」之外的所有标签字段（只改内存，执行任务时写入输出）。 */
  const clearOtherTags = () =>
    runBusy(async () => {
      if (effectiveIds.length === 0) {
        notify("info", "请先在左侧列表选择文件");
        return;
      }
      const edits = TAG_FIELDS.filter((field) => !KEEP_FIELDS.includes(field.id)).map((field) => ({
        field: field.id,
        value: null,
      }));
      await api.updateTags(effectiveIds, edits);
      await refreshItems();
      notify(
        "success",
        `已清除 ${effectiveIds.length} 个文件的 ${edits.length} 个标签字段（保留艺术家 / 标题）`,
      );
    });

  const runReplace = () =>
    runBusy(async () => {
      if (!find) {
        notify("info", "请填写要查找的内容");
        return;
      }
      await api.replaceInTags({ ids: effectiveIds, field: findField, find, replace: replaceWith, caseInsensitive });
      await refreshItems();
      notify("success", "查找替换完成");
    });

  const runGuess = () =>
    runBusy(async () => {
      await api.guessTags(effectiveIds, guessTemplate);
      await refreshItems();
      notify("success", "已根据文件名反推标签");
    });

  const embedCover = () =>
    runBusy(async () => {
      if (!single || !firstId) {
        notify("info", "请只选择一个文件再嵌入封面");
        return;
      }
      const image = await pickImageFile();
      if (!image) return;
      setCover(await api.setCover(firstId, image));
      await refreshItems();
      notify("success", "封面已就绪（执行任务时写入输出文件）");
    });

  const dropCover = () =>
    runBusy(async () => {
      if (!single || !firstId) {
        notify("info", "请只选择一个文件再删除封面");
        return;
      }
      await api.removeCover(firstId);
      setCover(null);
      await refreshItems();
      notify("success", "封面将在执行任务时移除");
    });

  const exportCover = () =>
    runBusy(async () => {
      if (!single || !firstId) {
        notify("info", "请只选择一个文件再导出封面");
        return;
      }
      const dir = await openDialog({ directory: true, multiple: false, title: "导出封面到目录" });
      if (typeof dir !== "string") return;
      const path = await api.exportCover(firstId, 0, dir);
      setExportHint(path);
      notify("success", "封面已导出");
    });

  return (
    <div className="flex flex-col gap-3">
      <Panel
        title="批量编辑标签"
        subtitle={`作用于 ${effectiveIds.length} 个文件 · 仅改内存，执行任务时写入`}
        icon="tag"
        actions={
          <>
            <Badge tone={enabledCount > 0 ? "accent" : "neutral"}>{enabledCount} 个字段</Badge>
            <Button
              size="sm"
              variant="ghost"
              icon="refresh"
              disabled={busy}
              onClick={() => {
                setEnabled({});
                const cleared: Record<string, string> = {};
                for (const field of TAG_FIELDS) cleared[field.id] = "";
                setValues(cleared);
              }}
            >
              清空
            </Button>
            <Button size="sm" variant="primary" icon="check" loading={busy} onClick={() => void applyEdits()}>
              应用到选中
            </Button>
            <Button
              size="sm"
              icon="trash"
              disabled={busy || effectiveIds.length === 0}
              title="一键清除除「艺术家 / 标题」之外的所有标签字段（不影响封面与音频；执行任务时才写入输出）"
              onClick={() => void clearOtherTags()}
            >
              清除其他标签
            </Button>
          </>
        }
      >
        <div className="grid grid-cols-2 gap-2">
          {TAG_FIELDS.map((field) => {
            const on = enabled[field.id] ?? false;
            return (
              <div
                key={field.id}
                className={`rounded-xl border p-2.5 transition-colors ${
                  on ? "border-accent/35 bg-accent/[0.06]" : "border-line bg-surface-2/60"
                }`}
              >
                <button
                  type="button"
                  onClick={() => setEnabled((prev) => ({ ...prev, [field.id]: !on }))}
                  className="mb-1.5 flex w-full items-center gap-2 text-left"
                >
                  <span
                    className={`grid h-[15px] w-[15px] shrink-0 place-items-center rounded-[4px] border transition-all ${
                      on ? "border-accent/70 bg-accent text-accent-fg" : "border-line-2 text-transparent"
                    }`}
                  >
                    <Icon name="check" size={10} />
                  </span>
                  <span className={`text-[11.5px] font-medium ${on ? "text-ink" : "text-ink-2"}`}>
                    {field.label}
                  </span>
                  {field.numeric && <span className="ml-auto text-[10px] text-ink-3">数字</span>}
                </button>
                <TextInput
                  value={values[field.id] ?? ""}
                  onChange={(value) => setValues((prev) => ({ ...prev, [field.id]: value }))}
                  placeholder={on ? (field.numeric ? "如 3（留空=清除）" : "留空 = 清除该字段") : "勾选后可编辑"}
                  disabled={!on}
                />
              </div>
            );
          })}
        </div>
        <p className="mt-2 text-[11px] text-ink-3">
          勾选字段并填写内容后点「应用到选中」，留空表示清除；「清除其他标签」保留艺术家与标题。修改在任务执行时写入输出文件。
        </p>
      </Panel>

      <Collapsible title="更多工具" hint="查找替换 / 文件名反推" icon="sparkle">
        <div className="grid grid-cols-2 gap-3">
          <Field label="字段">
            <Segmented
              value={findField}
              onChange={setFindField}
              options={[
                { value: "title", label: "标题" },
                { value: "artist", label: "艺术家" },
                { value: "album", label: "专辑" },
                { value: "genre", label: "流派" },
              ]}
              size="sm"
              className="w-full"
            />
          </Field>
          <Field label="查找">
            <TextInput value={find} onChange={setFind} placeholder="要查找的文本" />
          </Field>
          <Field label="替换为">
            <TextInput value={replaceWith} onChange={setReplaceWith} placeholder="留空 = 删除匹配文本" />
          </Field>
          <div className="flex items-end gap-3">
            <Switch checked={caseInsensitive} onChange={setCaseInsensitive} label="忽略大小写" />
            <Button size="sm" icon="check" disabled={busy} onClick={() => void runReplace()}>
              执行替换
            </Button>
          </div>
        </div>

        <div className="flex items-end gap-2">
          <div className="flex-1">
            <Field label="文件名模板">
              <TextInput value={guessTemplate} onChange={setGuessTemplate} mono placeholder="{artist} - {title}" />
            </Field>
          </div>
          <Button icon="sparkle" disabled={busy} onClick={() => void runGuess()}>
            反推
          </Button>
        </div>
      </Collapsible>
      <Panel
        title="封面"
        subtitle={single ? "嵌入 / 替换 / 导出当前文件的封面" : "请只选择一个文件"}
        icon="picture"
        actions={<Badge tone={single ? "accent" : "warn"}>{single ? "单选" : "多选不可用"}</Badge>}
      >
        <div className="flex gap-3">
          <div className="relative grid h-[132px] w-[132px] shrink-0 place-items-center overflow-hidden rounded-xl border border-line bg-surface-2">
            {cover ? (
              <img src={cover} alt="封面预览" className="h-full w-full object-cover" />
            ) : (
              <div className="flex flex-col items-center gap-1.5 text-ink-3">
                <Icon name="picture" size={22} />
                <span className="text-[11px]">无封面</span>
              </div>
            )}
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <Button size="sm" icon="picture" disabled={busy || !single} onClick={() => void embedCover()}>
              选择图片并嵌入
            </Button>
            <Button size="sm" icon="trash" disabled={busy || !single} onClick={() => void dropCover()}>
              删除封面
            </Button>
            <Button size="sm" icon="download" disabled={busy || !single} onClick={() => void exportCover()}>
              导出封面
            </Button>
            {exportHint && (
              <span className="truncate font-mono text-[10.5px] text-ink-3" title={exportHint}>
                {exportHint}
              </span>
            )}
          </div>
        </div>
      </Panel>
    </div>
  );
}
