// 顶部工具条：添加输入 + 批量操作（设计方案 §6.1）。

import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useApp } from "../store";
import { Button, Divider, IconButton, Switch } from "./ui";
import { Icon } from "./Icons";

export function Toolbar() {
  const {
    items,
    scan,
    clearList,
    removeItems,
    selected,
    ffmpegReady,
    pickInputPaths,
    notify,
  } = useApp();
  const [recursive, setRecursive] = useState(true);
  const [dragging, setDragging] = useState(false);
  const [dropCount, setDropCount] = useState(0);
  const [busy, setBusy] = useState(false);

  // 系统拖放必须走 Tauri 事件：WebView2 的 HTML5 drop 事件里 `File.path` 为空，拿不到本地路径。
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter") {
          setDragging(true);
          setDropCount(payload.paths.length);
        } else if (payload.type === "over") {
          setDragging(true);
        } else if (payload.type === "drop") {
          setDragging(false);
          if (payload.paths.length > 0) void scan(payload.paths, recursive);
          else notify("error", "拖入的内容没有可读取的路径");
        } else {
          setDragging(false);
        }
      })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch((error) => notify("error", error instanceof Error ? error.message : String(error)));
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [notify, recursive, scan]);

  const addFolder = async () => {
    const picked = await openDialog({ directory: true, multiple: true, title: "选择音乐文件夹" });
    if (typeof picked === "string") await scan([picked], recursive);
    else if (Array.isArray(picked)) await scan(picked, recursive);
  };

  const addFiles = async () => {
    const picked = await pickInputPaths();
    if (picked.length > 0) await scan(picked, false);
  };

  const withBusy = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="flex shrink-0 flex-wrap items-center gap-2.5 border-b border-line bg-surface-2/60 px-4 py-2.5">
        <Button
          variant="primary"
          icon="folderPlus"
          loading={busy}
          disabled={!ffmpegReady}
          onClick={() => void withBusy(addFolder)}
        >
          添加文件夹
        </Button>
        <Button
          icon="filePlus"
          disabled={!ffmpegReady || busy}
          onClick={() => void withBusy(addFiles)}
        >
          添加文件
        </Button>
        <Divider vertical />
        <Switch checked={recursive} onChange={setRecursive} label="含子目录" />

        <div className="ml-auto flex items-center gap-2">
          <IconButton
            icon="trash"
            title="移除选中（仅从列表移除）"
            size="sm"
            disabled={selected.length === 0}
            onClick={() => void removeItems(selected)}
          />
          <IconButton
            icon="x"
            title="清空列表"
            size="sm"
            disabled={items.length === 0}
            onClick={() => void clearList()}
          />
        </div>
      </div>

      {dragging && (
        <div className="pointer-events-none fixed inset-2 z-50 grid place-items-center rounded-2xl border-2 border-dashed border-accent/60 bg-accent/[0.08] backdrop-blur-sm">
          <div className="flex flex-col items-center gap-3 text-accent">
            <span className="grid h-16 w-16 place-items-center rounded-2xl bg-accent/15 ring-1 ring-accent/40">
              <Icon name="download" size={28} />
            </span>
            <span className="text-[14px] font-semibold">
              {dropCount > 1 ? `松开以添加这 ${dropCount} 个项目` : "松开以添加这些文件"}
            </span>
            <span className="text-[11.5px] text-accent/70">
              {recursive ? "将包含子目录" : "仅添加当前层级"}
            </span>
          </div>
        </div>
      )}
    </>
  );
}
