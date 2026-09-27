// 基础 UI 组件：语义色令牌（浅色/深色自适应）。

import type { ChangeEvent, ReactNode } from "react";
import { Icon, type IconName } from "./Icons";

const FOCUS =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:ring-offset-0";

export function Spinner({ size = 14, className = "" }: { size?: number; className?: string }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} className={`animate-spin ${className}`} aria-hidden="true">
      <circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
      <path d="M21 12a9 9 0 0 0-9-9" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

export function Button({
  children,
  onClick,
  variant = "default",
  size = "md",
  disabled,
  title,
  icon,
  loading,
  className = "",
}: {
  children?: ReactNode;
  onClick?: () => void;
  variant?: "default" | "primary" | "danger" | "ghost";
  size?: "sm" | "md";
  disabled?: boolean;
  title?: string;
  icon?: IconName;
  loading?: boolean;
  className?: string;
}) {
  const palette: Record<string, string> = {
    default:
      "bg-surface-2 text-ink ring-1 ring-line hover:bg-surface-3 hover:ring-line-2 active:translate-y-px",
    primary:
      "bg-gradient-to-b from-accent-fill-1 to-accent-fill-2 text-accent-fg font-semibold ring-1 ring-accent-fill-2/40 shadow-[0_8px_20px_-10px_var(--accent)] hover:brightness-[1.05] active:translate-y-px",
    danger:
      "bg-rose-soft text-rose-ink ring-1 ring-rose-ink/25 hover:brightness-[0.97] active:translate-y-px",
    ghost: "bg-transparent text-ink-2 hover:text-ink hover:bg-surface-3",
  };
  const sizing = size === "sm" ? "h-7 px-2.5 text-[12px] gap-1.5" : "h-[34px] px-3.5 text-[13px] gap-2";
  return (
    <button
      type="button"
      title={title}
      disabled={disabled || loading}
      onClick={onClick}
      className={`inline-flex shrink-0 items-center justify-center rounded-lg font-medium transition-all duration-150 ${palette[variant]} ${sizing} ${FOCUS} ${
        disabled || loading ? "cursor-not-allowed opacity-45" : ""
      } ${className}`}
    >
      {loading ? (
        <Spinner size={size === "sm" ? 12 : 14} />
      ) : icon ? (
        <Icon name={icon} size={size === "sm" ? 13 : 15} />
      ) : null}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  onClick,
  title,
  disabled,
  tone = "default",
  size = "md",
  active,
}: {
  icon: IconName;
  onClick?: () => void;
  title: string;
  disabled?: boolean;
  tone?: "default" | "danger" | "accent";
  size?: "sm" | "md";
  active?: boolean;
}) {
  const tones: Record<string, string> = {
    default: "text-ink-2 hover:text-ink hover:bg-surface-3 hover:ring-line-2",
    danger: "text-ink-3 hover:text-rose-ink hover:bg-rose-soft hover:ring-rose-ink/25",
    accent: "text-accent hover:bg-accent-soft hover:ring-accent/30",
  };
  const box = size === "sm" ? "h-7 w-7" : "h-[34px] w-[34px]";
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={`grid ${box} shrink-0 place-items-center rounded-lg border border-line transition-all duration-150 ${tones[tone]} ${FOCUS} ${
        active ? "bg-accent-soft text-accent ring-1 ring-accent/30" : "bg-surface-2"
      } ${disabled ? "cursor-not-allowed opacity-40" : ""}`}
    >
      <Icon name={icon} size={size === "sm" ? 14 : 16} />
    </button>
  );
}

export function Panel({
  title,
  subtitle,
  icon,
  children,
  actions,
  className = "",
}: {
  title?: string;
  subtitle?: string;
  icon?: IconName;
  children: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <section className={`rounded-2xl border border-line bg-surface shadow-[var(--shadow-card)] ${className}`}>
      {(title || actions) && (
        <header className="flex items-start justify-between gap-3 border-b border-line px-3.5 py-2.5">
          <div className="flex min-w-0 items-center gap-2.5">
            {icon && (
              <span className="grid h-7 w-7 place-items-center rounded-lg bg-accent-soft text-accent ring-1 ring-accent/15">
                <Icon name={icon} size={15} />
              </span>
            )}
            <div className="min-w-0">
              <h2 className="truncate text-[13px] font-semibold tracking-wide text-ink">{title}</h2>
              {subtitle && <p className="truncate text-[11px] text-ink-3">{subtitle}</p>}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">{actions}</div>
        </header>
      )}
      <div className="p-3.5">{children}</div>
    </section>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="block min-w-0">
      <span className="mb-1.5 flex items-baseline gap-1.5">
        <span className="text-[11px] font-medium text-ink-2">{label}</span>
        {hint && <span className="truncate text-[11px] text-ink-3">{hint}</span>}
      </span>
      {children}
    </label>
  );
}

const INPUT =
  "h-9 w-full rounded-lg border border-line bg-surface-2 px-2.5 text-[13px] text-ink outline-none transition-colors placeholder:text-ink-3 hover:border-line-2 focus:border-accent focus:bg-surface disabled:cursor-not-allowed disabled:opacity-50";

export function TextInput({
  value,
  onChange,
  placeholder,
  mono,
  disabled,
  onEnter,
  prefixIcon,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  mono?: boolean;
  disabled?: boolean;
  onEnter?: () => void;
  prefixIcon?: IconName;
}) {
  const input = (
    <input
      type="text"
      value={value}
      placeholder={placeholder}
      spellCheck={false}
      disabled={disabled}
      onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && onEnter) onEnter();
      }}
      className={`${INPUT} ${mono ? "font-mono text-[12px]" : ""} ${prefixIcon ? "pl-8" : ""}`}
    />
  );
  if (!prefixIcon) return input;
  return (
    <div className="relative">
      <Icon name={prefixIcon} size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-3" />
      {input}
    </div>
  );
}

export function NumberInput({
  value,
  onChange,
  step = 1,
  min,
  max,
  placeholder,
  disabled,
  suffix,
}: {
  value: number | null;
  onChange: (value: number | null) => void;
  step?: number;
  min?: number;
  max?: number;
  placeholder?: string;
  disabled?: boolean;
  suffix?: string;
}) {
  return (
    <div className="relative">
      <input
        type="number"
        value={value === null ? "" : value}
        step={step}
        min={min}
        max={max}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(e: ChangeEvent<HTMLInputElement>) => {
          const raw = e.target.value;
          if (raw === "") return onChange(null);
          const parsed = Number(raw);
          onChange(Number.isFinite(parsed) ? parsed : null);
        }}
        className={`${INPUT} tnum font-mono text-[12px] ${suffix ? "pr-14" : ""}`}
      />
      {suffix && (
        <span className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-[11px] text-ink-3">
          {suffix}
        </span>
      )}
    </div>
  );
}

export function Select({
  value,
  onChange,
  options,
  disabled,
}: {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string; disabled?: boolean }[];
  disabled?: boolean;
}) {
  return (
    <div className="relative">
      <select
        value={value}
        disabled={disabled}
        onChange={(e: ChangeEvent<HTMLSelectElement>) => onChange(e.target.value)}
        className={`${INPUT} cursor-pointer appearance-none pr-8`}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      <Icon
        name="chevronDown"
        size={15}
        className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-ink-3"
      />
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  hint,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  hint?: string;
  disabled?: boolean;
}) {
  return (
    <label className={`flex cursor-pointer select-none items-center gap-2.5 ${disabled ? "cursor-not-allowed opacity-45" : ""}`}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative h-5 w-9 shrink-0 rounded-full border transition-colors duration-200 ${FOCUS} ${
          checked ? "border-accent bg-accent" : "border-line-2 bg-surface-3"
        }`}
      >
        <span
          className={`absolute top-[2px] h-[15px] w-[15px] rounded-full bg-white shadow transition-all duration-200 ${
            checked ? "left-[19px]" : "left-[2px]"
          }`}
        />
      </button>
      <span className="min-w-0">
        <span className="block text-[12.5px] text-ink">{label}</span>
        {hint && <span className="block text-[11px] text-ink-3">{hint}</span>}
      </span>
    </label>
  );
}

export function Segmented<T extends string>({
  value,
  onChange,
  options,
  size = "md",
  className = "",
}: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string; icon?: IconName }[];
  size?: "sm" | "md";
  className?: string;
}) {
  const box = size === "sm" ? "h-7 px-2 text-[11.5px]" : "h-8 px-2.5 text-[12.5px]";
  return (
    <div className={`inline-flex rounded-lg border border-line bg-surface-2 p-[3px] ${className}`}>
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            onClick={() => onChange(option.value)}
            className={`inline-flex flex-1 items-center justify-center gap-1.5 rounded-[6px] font-medium transition-all duration-150 ${box} ${FOCUS} ${
              active
                ? "bg-surface text-ink shadow-[0_1px_2px_rgba(15,23,42,0.12)] ring-1 ring-line-2"
                : "text-ink-3 hover:text-ink"
            }`}
          >
            {option.icon && <Icon name={option.icon} size={13} />}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export function Progress({
  value,
  label,
  detail,
  running = true,
}: {
  value: number;
  label?: string;
  detail?: string;
  running?: boolean;
}) {
  const percent = Math.max(0, Math.min(100, value * 100));
  return (
    <div className="w-full">
      <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-surface-3 ring-1 ring-inset ring-line">
        <div
          className="h-full rounded-full bg-gradient-to-r from-accent/45 via-accent to-accent shadow-[0_0_10px_var(--accent-soft)] transition-[width] duration-300 ease-out"
          style={{ width: `${percent}%` }}
        />
        {running && percent > 0 && percent < 100 && (
          <div className="progress-stripes absolute inset-y-0 left-0 rounded-full" style={{ width: `${percent}%` }} />
        )}
      </div>
      {(label || detail) && (
        <div className="mt-1.5 flex items-baseline gap-2 text-[11px]">
          <span className="truncate text-ink-2" title={label}>
            {label}
          </span>
          {detail && <span className="shrink-0 text-ink-3">{detail}</span>}
          <span className="tnum ml-auto shrink-0 font-mono text-ink">{percent.toFixed(0)}%</span>
        </div>
      )}
    </div>
  );
}

export type Tone = "neutral" | "ok" | "warn" | "err" | "accent" | "iris";

export function Badge({ children, tone = "neutral", icon }: { children: ReactNode; tone?: Tone; icon?: IconName }) {
  const tones: Record<Tone, string> = {
    neutral: "bg-surface-2 text-ink-2 ring-line",
    ok: "bg-mint-soft text-mint ring-mint/25",
    warn: "bg-amber-soft text-amber-ink ring-amber-ink/25",
    err: "bg-rose-soft text-rose-ink ring-rose-ink/25",
    accent: "bg-accent-soft text-accent ring-accent/25",
    iris: "bg-iris-soft text-iris ring-iris/25",
  };
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md px-1.5 py-[3px] text-[11px] font-medium leading-none ring-1 ring-inset ${tones[tone]}`}
    >
      {icon && <Icon name={icon} size={11} />}
      {children}
    </span>
  );
}

export function Stat({
  label,
  value,
  tone = "neutral",
  icon,
}: {
  label: string;
  value: string | number;
  tone?: Tone;
  icon?: IconName;
}) {
  const color: Record<Tone, string> = {
    neutral: "text-ink",
    ok: "text-mint",
    warn: "text-amber-ink",
    err: "text-rose-ink",
    accent: "text-accent",
    iris: "text-iris",
  };
  return (
    <span className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-2 py-1">
      {icon && <Icon name={icon} size={12} className={color[tone]} />}
      <span className="text-[11px] text-ink-3">{label}</span>
      <span className={`tnum font-mono text-[12px] font-semibold ${color[tone]}`}>{value}</span>
    </span>
  );
}

export function EmptyHint({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full min-h-[180px] flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-line-2 bg-surface-2/50 px-6 py-10 text-center">
      <span className="grid h-14 w-14 place-items-center rounded-2xl bg-surface text-ink-3 ring-1 ring-line">
        <Icon name="waveform" size={26} />
      </span>
      <div className="max-w-[340px] text-[12.5px] leading-relaxed text-ink-3">{children}</div>
    </div>
  );
}

export function Divider({ vertical = false }: { vertical?: boolean }) {
  return vertical ? <span className="mx-0.5 h-5 w-px shrink-0 bg-line" /> : <span className="my-1 block h-px w-full bg-line" />;
}

/** 折叠区：把低频高级选项收起来，默认只展示核心选项。 */
export function Collapsible({
  title,
  hint,
  icon,
  defaultOpen = false,
  children,
}: {
  title: string;
  hint?: string;
  icon?: IconName;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  return (
    <details className="group rounded-2xl border border-line bg-surface shadow-[var(--shadow-card)]" open={defaultOpen}>
      <summary className="flex cursor-pointer select-none items-center gap-2 px-3.5 py-2.5 text-[12.5px] font-medium text-ink-2 transition-colors hover:text-ink [&::-webkit-details-marker]:hidden">
        <Icon name="chevronRight" size={13} className="shrink-0 text-ink-3 transition-transform group-open:rotate-90" />
        {icon && <Icon name={icon} size={14} className="shrink-0 text-ink-3" />}
        <span>{title}</span>
        {hint && <span className="ml-auto truncate text-[11px] font-normal text-ink-3">{hint}</span>}
      </summary>
      <div className="flex flex-col gap-3 border-t border-line px-3.5 py-3">{children}</div>
    </details>
  );
}

// ------------------------------------------------------------------ 格式化

export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "—";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function formatDb(value: number | null, suffix = " dB"): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return `${value.toFixed(2)}${suffix}`;
}

export function formatBytes(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let index = 0;
  let value = bytes;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index++;
  }
  return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

export const RATING_LABEL: Record<string, { text: string; tone: Tone }> = {
  hi_res: { text: "Hi-Res", tone: "accent" },
  lossless: { text: "无损", tone: "ok" },
  lossy: { text: "有损", tone: "warn" },
  dsd: { text: "DSD", tone: "iris" },
};

export const STATUS_LABEL: Record<string, { text: string; tone: Tone }> = {
  success: { text: "成功", tone: "ok" },
  skipped: { text: "跳过", tone: "warn" },
  failed: { text: "失败", tone: "err" },
  cancelled: { text: "已取消", tone: "neutral" },
};
