import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { AppProvider } from "./store";
import { applyTheme, getStoredTheme } from "./theme";
import "./styles.css";

// 先应用主题，避免首帧闪烁
applyTheme(getStoredTheme(), false);

const container = document.getElementById("root");
if (!container) throw new Error("找不到 #root 容器");

createRoot(container).render(
  <StrictMode>
    <AppProvider>
      <App />
    </AppProvider>
  </StrictMode>,
);
