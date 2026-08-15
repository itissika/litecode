import { useEffect, useState, useCallback, useMemo } from "react";
import { DockviewReact } from "dockview-react";

import { panelComponents, tabComponents } from "./config/registry";
import { keyboardKeymap } from "./config/keymap";
import { useDockviewConfig } from "./hooks/useDockviewConfig";
import { ensureSearchPanel } from "./config/layout";
import { TitleBar } from "./shell/TitleBar";
import { StatusBar } from "../components/StatusBar";
import { WelcomeWatermark } from "./watermark/WelcomeWatermark";
import { FloatingDialog } from "./components/FloatingDialog";
import { DOCKVIEW_CLASS, dockviewThemeForApp } from "../theme/dockview/adapter";
import { getTheme, setTheme, THEME_CHANGE_EVENT, type ThemeName } from "../lib/theme";
import { SettingsDialog } from "./panels/SettingsDialog";
import { useSettingsStore } from "../stores/settingsStore";
import { AboutContent } from "./panels/AboutPanel";
import { Logo } from "../components/Logo";
import { useEditorStore } from "../stores/editorStore";
import "../lib/litecodeTerminal";

function readSessionMode(): "local" | "remote" {
  return window.litecode?.getSessionMode?.() === "remote" ? "remote" : "local";
}

export function AppShellDockview() {
  const { onReady, onWillDrop, getTabContextMenuItems } = useDockviewConfig();
  const [dialogVisible, setDialogVisible] = useState(false);
  const [aboutReplay, setAboutReplay] = useState(0);
  const [sessionMode, setSessionMode] = useState<"local" | "remote">(readSessionMode);
  const [appTheme, setAppTheme] = useState<ThemeName>(() => getTheme());
  const dockviewTheme = useMemo(() => dockviewThemeForApp(appTheme), [appTheme]);

  const openSettings = useSettingsStore((s) => s.openSettings);

  useEffect(() => {
    setSessionMode(readSessionMode());
  }, []);

  useEffect(() => {
    const onTheme = (e: Event) => {
      const name = (e as CustomEvent<string>).detail;
      setAppTheme(name === "light" ? "light" : "default");
    };
    window.addEventListener(THEME_CHANGE_EVENT, onTheme);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, onTheme);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        const api = useEditorStore.getState().dockviewApi;
        if (!api) return;
        ensureSearchPanel(api);
        api.getPanel("workspace-search")?.api.setActive();
        window.dispatchEvent(new Event("litecode:focus-workspace-search"));
      } else if ((e.ctrlKey || e.metaKey) && (e.key === "s" || e.key === "S")) {
        // Global "save active editor" — mirrors VS Code's workbench-level
        // Ctrl+S (no focus/active-panel gating). save() defaults to activePath.
        e.preventDefault();
        void useEditorStore.getState().save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const handleMenuAction = useCallback((item: string) => {
    if (item === "Home") {
      void window.litecode?.returnToHub?.();
    } else if (item === "About") {
      setAboutReplay((n) => n + 1);
      setDialogVisible(true);
    } else if (item === "Settings...") {
      openSettings();
    } else if (item === "Theme: Dark") {
      setTheme("default");
    } else if (item === "Theme: Light") {
      setTheme("light");
    }
  }, [openSettings]);

  return (
    <>
      <div className="litecode-dv-base relative flex h-[100dvh] w-screen flex-col overflow-hidden">
        <Logo size="lg" splash />
        <TitleBar onMenuAction={handleMenuAction} sessionMode={sessionMode} />

        <div className="relative flex-1 min-h-0 overflow-hidden">
          <DockviewReact
            components={panelComponents}
            tabComponents={tabComponents}
            watermarkComponent={WelcomeWatermark}
            onReady={onReady}
            className={DOCKVIEW_CLASS}
            theme={dockviewTheme}
            keyboardNavigation={{ keymap: keyboardKeymap }}
            onWillDrop={onWillDrop}
            getTabContextMenuItems={getTabContextMenuItems}
            getTabGroupChipContextMenuItems={() => ["rename", "colorPicker"]}
            defaultRenderer="always"
          />
        </div>

        <StatusBar sessionMode={sessionMode} />

        <FloatingDialog
          visible={dialogVisible}
          title="About"
          onClose={() => setDialogVisible(false)}
        >
          <AboutContent replay={aboutReplay} />
        </FloatingDialog>
        <SettingsDialog />
      </div>
    </>
  );
}
