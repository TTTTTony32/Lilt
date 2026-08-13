import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SelectionView from "./SelectionView";
import "./styles.css";

const isSelectionWindow = getCurrentWindow().label === "selection";
if (isSelectionWindow) document.body.classList.add("selection-body");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isSelectionWindow ? <SelectionView /> : <App />}
  </StrictMode>,
);
