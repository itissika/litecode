import "@fontsource/inter/400.css";
import "@fontsource/jetbrains-mono/400.css";
import "dockview-react/dist/styles/dockview.css";
import "./theme/index.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import "./i18n";
import { whenIdle } from "./lib/idle";
import { installCitationCopy } from "./lib/knowledge/citationCopy";
import { startSessionActivityBridge } from "./lib/sessionActivity";
import { applyTheme, getTheme } from "./lib/theme";

// Apply persisted theme before first render to avoid flash
applyTheme(getTheme());

// Copying a reply must carry its citations: the chips are atomic references and
// sit on `user-select: none`, so a plain selection would drop them.
installCitationCopy();

// A running turn dies with the window; the host asks before that happens.
startSessionActivityBridge();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Only the 400 faces block first paint; the other weights load when idle.
void whenIdle().then(() => import("./fontsDeferred"));
