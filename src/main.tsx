import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SelectionView from "./SelectionView";
import { invokeCommand } from "./lib/tauri";
import "./styles.css";

const isSelectionWindow = getCurrentWindow().label === "selection";
if (isSelectionWindow) {
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  document.getElementById("root")?.style.setProperty("background", "transparent");
  document.body.classList.add("selection-body");
}
if (!isSelectionWindow) {
  void invokeCommand("report_startup_stage", { stage: "webview_script" }).catch(() => undefined);
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isSelectionWindow ? <SelectionView /> : <App />}
  </StrictMode>,
);

if (!isSelectionWindow) {
  window.requestAnimationFrame(() => {
    void invokeCommand("report_startup_stage", { stage: "dom_mounted" }).catch(() => undefined);
    window.requestAnimationFrame(() => {
      void invokeCommand("report_startup_stage", { stage: "first_paint" }).catch(() => undefined);
    });
  });
}
