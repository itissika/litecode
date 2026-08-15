import { Link, Cube, WrenchIcon, Robot, GearSix, Cpu, Code } from "@phosphor-icons/react";

import { useSettingsStore, type SettingsSection } from "../../stores/settingsStore";
import { SettingsSkeleton } from "../../components/ui/Skeleton";
import { FloatingDialog } from "../components/FloatingDialog";
import { ConnectionSection } from "./settings/ConnectionSection";
import { ModelsSection } from "./settings/ModelsSection";
import { ToolCatalogSection } from "./settings/ToolCatalogSection";
import { CustomToolsSection } from "./settings/CustomToolsSection";
import { AgentsSection } from "./settings/AgentsSection";
import { AdvancedSection } from "./settings/AdvancedSection";
import { EnginesSection } from "./settings/engines/EnginesSection";

const SECTIONS: { id: SettingsSection; label: string; Icon: React.FC<{ size?: number }> }[] = [
  { id: "connection", label: "Provider", Icon: Link },
  { id: "models", label: "Models", Icon: Cube },
  { id: "engines", label: "Engines", Icon: Cpu },
  { id: "tool-catalog", label: "Tool Catalog", Icon: WrenchIcon },
  { id: "custom-tools", label: "Custom Tools", Icon: Code },
  { id: "agents", label: "Agents", Icon: Robot },
  { id: "advanced", label: "Advanced", Icon: GearSix },
];

function TurnBlockedBanner() {
  const blocked = useSettingsStore((s) => s.isSaveBlocked());
  if (!blocked) return null;
  return (
    <div className="border-b border-(--_dk-amber-900\/20) bg-(--_dk-amber-900\/20) px-4 py-2 text-sm text-(--_dk-amber-500)">
      Agent is busy - settings saves are disabled until the chat view is stable.
    </div>
  );
}

function SectionContent({ section }: { section: SettingsSection }) {
  switch (section) {
    case "connection":
      return <ConnectionSection />;
    case "models":
      return <ModelsSection />;
    case "engines":
      return <EnginesSection />;
    case "tool-catalog":
      return <ToolCatalogSection />;
    case "custom-tools":
      return <CustomToolsSection />;
    case "agents":
      return <AgentsSection />;
    case "advanced":
      return <AdvancedSection />;
  }
}

export function SettingsDialog() {
  const open = useSettingsStore((s) => s.open);
  const section = useSettingsStore((s) => s.section);
  const revision = useSettingsStore((s) => s.revision);
  const loading = useSettingsStore((s) => s.loading);
  const loadError = useSettingsStore((s) => s.loadError);
  const closeSettings = useSettingsStore((s) => s.closeSettings);
  const setSection = useSettingsStore((s) => s.setSection);

  return (
    <FloatingDialog
      visible={open}
      title={`Settings — rev ${revision}`}
      onClose={closeSettings}
      defaultWidth={860}
      defaultHeight={640}
    >
      <div className="flex flex-col h-full">
        <TurnBlockedBanner />

        <div className="flex min-h-0 flex-1">
          <nav className="shrink-0 bg-(--_dk-side)">
            {SECTIONS.map((item) => {
              const Icon = item.Icon;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => setSection(item.id)}
                  className={
                    section === item.id
                      ? "settings-nav-item settings-nav-item-active bg-(--_dk-overlay)"
                      : "settings-nav-item"
                  }
                >
                  <span className="settings-nav-item-content">
                    <Icon size={14} />
                    <span>{item.label}</span>
                  </span>
                </button>
              );
            })}
          </nav>

          <div className="min-w-0 flex-1 bg-(--_dk-overlay)">
            <div key={section} className="settings-content-enter h-full">
              {loading ? (
                <div className="h-full overflow-y-auto px-6 py-5">
                  <SettingsSkeleton />
                </div>
              ) : loadError ? (
                <p className="px-6 py-5 text-sm text-(--_dk-red-500)">{loadError}</p>
              ) : (
                <SectionContent section={section} />
              )}
            </div>
          </div>
        </div>
      </div>
    </FloatingDialog>
  );
}
