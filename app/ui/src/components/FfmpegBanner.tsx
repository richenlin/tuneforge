// 未定位到 ffmpeg 时的阻断提示（设计方案 §11）。

import { useApp } from "../store";
import { Button } from "./ui";
import { Icon } from "./Icons";

export function FfmpegBanner() {
  const { ffmpeg, ffmpegReady, ffmpegProbing, pickFfmpegDir, refreshFfmpeg } = useApp();
  // 探测中不报警（顶栏胶囊已有“检测中”提示，且界面可正常使用）
  if (ffmpegReady || ffmpegProbing) return null;

  return (
    <div className="px-3 pt-3">
      <div className="animate-fade flex items-start gap-3 rounded-2xl border border-amber-ink/25 bg-amber-soft p-3.5">
        <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-amber-soft text-amber-ink ring-1 ring-amber-ink/25">
          <Icon name="alert" size={17} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-semibold text-ink">未找到可用的 ffmpeg / ffprobe</div>
          <p className="mt-0.5 text-[11.5px] leading-relaxed text-ink-2">
            {ffmpeg?.message ??
              "请把 ffmpeg 与 ffprobe 放到随包目录，或指定它们所在的位置（设置会立即生效并缓存）。"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="primary" size="sm" icon="folder" onClick={() => void pickFfmpegDir()}>
            指定目录
          </Button>
          <Button size="sm" icon="refresh" onClick={() => void refreshFfmpeg()}>
            重新检测
          </Button>
        </div>
      </div>
    </div>
  );
}
