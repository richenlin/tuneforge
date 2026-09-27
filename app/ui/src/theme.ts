// 主题：默认深色（黑 / 灰背景），可切换到浅色；写入 localStorage 并在启动时应用。
// 注：storage key 带 v2 —— 改版后深色为基线，旧的浅色偏好不再覆盖默认值。

export type Theme = "light" | "dark";

const KEY = "tuneforge.theme.v2";

export function getStoredTheme(): Theme {
  try {
    return localStorage.getItem(KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function applyTheme(theme: Theme, persist = true): void {
  document.documentElement.dataset.theme = theme;
  if (persist) {
    try {
      localStorage.setItem(KEY, theme);
    } catch {
      // 忽略存储失败（隐私模式等）
    }
  }
}
