import "@fontsource/inter/400.css";
import "@fontsource/jetbrains-mono/400.css";
import "dockview-react/dist/styles/dockview.css";
import "./theme/index.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { whenIdle } from "./lib/idle";
import { applyTheme, getTheme } from "./lib/theme";

// Apply persisted theme before first render to avoid flash
applyTheme(getTheme());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Only the 400 faces block first paint; the other weights load when idle.
void whenIdle().then(() => import("./fontsDeferred"));
