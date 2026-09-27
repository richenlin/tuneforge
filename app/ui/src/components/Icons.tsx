// 内联 SVG 图标集（无第三方依赖，统一 1.7 线宽 / 24 网格）。

import type { SVGProps } from "react";

const circle = (cx: number, cy: number, r: number) =>
  `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`;

const ICONS = {
  folder: ["M3 7.5a2 2 0 0 1 2-2h3.3l1.8 2.2H19a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
  folderPlus: [
    "M3 7.5a2 2 0 0 1 2-2h3.3l1.8 2.2H19a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    "M12 11.5v5M9.5 14h5",
  ],
  filePlus: [
    "M14 3.5H7.5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8z",
    "M14 3.5V8h4.5",
    "M12 12.5v4.5M9.8 14.8h4.4",
  ],
  fileText: [
    "M14 3.5H7.5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8z",
    "M14 3.5V8h4.5",
    "M9.5 12.5h5M9.5 16h5",
  ],
  music: [
    "M9 17.5V6.2l9.5-1.9v11.2",
    circle(6.6, 17.6, 2.4),
    circle(16.1, 15.6, 2.4),
  ],
  convert: ["M4 8.5h12.5l-3.2-3.2", "M20 15.5H7.5l3.2 3.2"],
  pencil: ["M4 20h4l10-10-4-4L4 16z", "M13.5 6.5l4 4"],
  sliders: ["M4 7.5h6M14.5 7.5H20M4 12h3M11.5 12H20M4 16.5h7M15.5 16.5H20", circle(12, 7.5, 2.2), circle(9.2, 12, 2.2), circle(13, 16.5, 2.2)],
  tag: [
    "M20.4 13.2l-7 7a2 2 0 0 1-2.8 0l-7-7A2 2 0 0 1 3 11.8V5a2 2 0 0 1 2-2h6.8a2 2 0 0 1 1.4.6l7.2 7.2a2 2 0 0 1 0 2.4z",
    circle(7.6, 7.6, 1.3),
  ],
  picture: ["M4 5.5h16v13H4z", circle(9, 10, 1.7), "M4 16.5l4.6-4.4 3 3L15.3 11l4.7 5"],
  play: ["M7.5 5.2l11 6.8-11 6.8z"],
  stop: ["M7 7h10v10H7z"],
  check: ["M5 12.8l4.4 4.4L19 6.6"],
  checkCircle: [circle(12, 12, 8.6), "M8.2 12.4l2.6 2.6 5-5.4"],
  x: ["M6.5 6.5l11 11M17.5 6.5l-11 11"],
  xCircle: [circle(12, 12, 8.6), "M9.2 9.2l5.6 5.6M14.8 9.2l-5.6 5.6"],
  alert: ["M12 3.6l8.8 15.8H3.2z", "M12 9.4v4.4M12 16.6h.01"],
  info: [circle(12, 12, 8.6), "M12 11.2v5M12 8.2h.01"],
  refresh: ["M20 12a8 8 0 1 1-2.6-5.9", "M20 3.8v5h-5"],
  trash: ["M4.5 7.5h15", "M9.5 7.5V5h5v2.5", "M6.6 7.5l.9 12.2h9l.9-12.2"],
  search: [circle(11, 11, 6), "M20 20l-4.4-4.4"],
  external: ["M14 4.5h5.5V10", "M19.5 4.5L11 13", "M18 14.5v4.5a1 1 0 0 1-1 1H5.5a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1H10"],
  download: ["M12 4.5v10.5", "M7.8 11.2L12 15.4l4.2-4.2", "M5 19.5h14"],
  upload: ["M12 15.5V5", "M7.8 9.2L12 5l4.2 4.2", "M5 19.5h14"],
  terminal: ["M5.5 6.5l5.5 5.5-5.5 5.5", "M13 17.5h6"],
  list: ["M8.5 6.5H20M8.5 12H20M8.5 17.5H20", circle(4.6, 6.5, 1.1), circle(4.6, 12, 1.1), circle(4.6, 17.5, 1.1)],
  gauge: ["M3.8 18a8.2 8.2 0 1 1 16.4 0", "M12 18l3.6-5.6", circle(12, 18, 1.2)],
  sparkle: ["M5 19l8.5-8.5", "M15 4.5l1 2.4 2.4 1-2.4 1-1 2.4-1-2.4-2.4-1 2.4-1z"],
  settings: [circle(12, 12, 3.1), "M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M5.9 5.9l1.6 1.6M16.5 16.5l1.6 1.6M18.1 5.9l-1.6 1.6M7.5 16.5l-1.6 1.6"],
  waveform: ["M4 11.5v1.5M8 8v8M12 5v14M16 9v6M20 11.5v1.5"],
  chevronDown: ["M6.5 9.5l5.5 5.5 5.5-5.5"],
  chevronRight: ["M9.5 6l6 6-6 6"],
  clock: [circle(12, 12, 8.6), "M12 7.5V12l3.2 2"],
  sun: [circle(12, 12, 4), "M12 2.6v2.2M12 19.2v2.2M2.6 12h2.2M19.2 12h2.2M5.4 5.4l1.6 1.6M17 17l1.6 1.6M18.6 5.4L17 7M7 17l-1.6 1.6"],
  moon: ["M20 14.4A8.4 8.4 0 0 1 9.6 4a8.4 8.4 0 1 0 10.4 10.4z"],
  layers: ["M12 4l8 4-8 4-8-4z", "M4.5 12.5L12 16.4l7.5-3.9", "M4.5 16.4L12 20.3l7.5-3.9"],
} as const;

export type IconName = keyof typeof ICONS;

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  name: IconName;
  size?: number;
  filled?: boolean;
}

export function Icon({ name, size = 16, filled = false, className = "", ...rest }: IconProps) {
  const paths = ICONS[name] as readonly string[];
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={filled ? 0 : 1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${className}`}
      aria-hidden="true"
      {...rest}
    >
      {paths.map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}

/** 应用标识：圆角方块 + 等化器柱（与应用图标一致）。 */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} className="shrink-0">
      <defs>
        <linearGradient id="tf-bg" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#1e293b" />
          <stop offset="100%" stopColor="#0b1220" />
        </linearGradient>
        <linearGradient id="tf-bar" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#22d3ee" />
          <stop offset="100%" stopColor="#4ade80" />
        </linearGradient>
      </defs>
      <rect x="1" y="1" width="30" height="30" rx="8" fill="url(#tf-bg)" stroke="rgba(148,163,184,0.25)" />
      <g fill="url(#tf-bar)">
        <rect x="7" y="18" width="3" height="6" rx="1.5" />
        <rect x="12" y="13" width="3" height="11" rx="1.5" />
        <rect x="17" y="8" width="3" height="16" rx="1.5" />
        <rect x="22" y="15" width="3" height="9" rx="1.5" />
      </g>
    </svg>
  );
}
