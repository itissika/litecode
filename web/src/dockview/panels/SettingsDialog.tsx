import { Link, Cube, Robot, GearSix, Cpu, Code, PlugsConnected, Tree } from "@phosphor-icons/react";

import { useSettingsStore, type SettingsSection } from "../../stores/settingsStore";
import { SettingsSkeleton } from "../../components/ui/Skeleton";
import { FloatingDialog } from "../components/FloatingDialog";
import { ConnectionSection } from "./settings/ConnectionSection";
import { ModelsSection } from "./settings/ModelsSection";
import { CustomToolsSection } from "./settings/CustomToolsSection";
import { McpServersSection } from "./settings/McpServersSection";
import { AgentsSection } from "./settings/AgentsSection";
import { AdvancedSection } from "./settings/AdvancedSection";
import { FilesSection } from "./settings/FilesSection";
import { EnginesSection } from "./settings/engines/EnginesSection";

const NAV_GROUPS: {
  title: string;
  items: { id: SettingsSection; label: string; Icon: React.FC<{ size?: number }> }[];
}[] = [
  {
    title: "LLM",
    items: [
      { id: "connection", label: "Provider", Icon: Link },
      { id: "models", label: "Models", Icon: Cube },
    ],
  },
  {
    title: "Agent",
    items: [
      { id: "agents", label: "Agents", Icon: Robot },
      { id: "custom-tools", label: "Custom Tools", Icon: Code },
      { id: "mcp", label: "MCP", Icon: PlugsConnected },
    ],
  },
  {
    title: "System",
    items: [
      { id: "engines", label: "Engines", Icon: Cpu },
      { id: "files", label: "Files", Icon: Tree },
      { id: "advanced", label: "Advanced", Icon: GearSix },
    ],
  },
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
    case "files":
      return <FilesSection />;
    case "custom-tools":
      return <CustomToolsSection />;
    case "mcp":
      return <McpServersSection />;
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
      onClose={() => void closeSettings()}
      defaultWidth={860}
      defaultHeight={640}
    >
      <div className="flex h-full flex-col">
        <TurnBlockedBanner />

        <div className="flex min-h-0 flex-1">
          <nav className="settings-nav shrink-0 bg-(--_dk-side)">
            {NAV_GROUPS.map((group) => (
              <div key={group.title} className="settings-nav-group">
                <div className="settings-nav-group-title">{group.title}</div>
                {group.items.map((item) => {
                  const Icon = item.Icon;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => void setSection(item.id)}
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
              </div>
            ))}
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
