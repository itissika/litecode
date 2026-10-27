import {
  Link,
  Cube,
  Robot,
  GearSix,
  Cpu,
  Code,
  PlugsConnected,
  Tree,
} from "@phosphor-icons/react";

import {
  useSettingsStore,
  sectionNeedsSkeleton,
  type SettingsSection,
} from "../../stores/settingsStore";
import { SettingsSkeleton } from "../../components/ui/Skeleton";
import { FloatingDialog } from "../components/FloatingDialog";
import { ProvidersSection } from "./settings/ProvidersSection";
import { ModelsSection } from "./settings/ModelsSection";
import { CustomToolsSection } from "./settings/CustomToolsSection";
import { McpServersSection } from "./settings/McpServersSection";
import { AgentsSection } from "./settings/AgentsSection";
import { AdvancedSection } from "./settings/AdvancedSection";
import { FilesSection } from "./settings/FilesSection";
import { EnginesSection } from "./settings/engines/EnginesSection";
import { useSettingsSaveBlocked } from "./settings/shared";
import i18n from "../../i18n";

const NAV_GROUPS: {
  titleKey: string;
  items: {
    id: SettingsSection;
    labelKey: string;
    Icon: React.FC<{ size?: number }>;
  }[];
}[] = [
  {
    titleKey: "settings.nav.llm",
    items: [
      { id: "connection", labelKey: "settings.nav.provider", Icon: Link },
      { id: "models", labelKey: "settings.nav.models", Icon: Cube },
    ],
  },
  {
    titleKey: "settings.nav.agentGroup",
    items: [
      { id: "agents", labelKey: "settings.agents.title", Icon: Robot },
      { id: "custom-tools", labelKey: "settings.nav.customTools", Icon: Code },
      { id: "mcp", labelKey: "settings.nav.mcp", Icon: PlugsConnected },
    ],
  },
  {
    titleKey: "settings.nav.system",
    items: [
      { id: "engines", labelKey: "settings.nav.engines", Icon: Cpu },
      { id: "files", labelKey: "settings.nav.files", Icon: Tree },
      { id: "advanced", labelKey: "settings.nav.advanced", Icon: GearSix },
    ],
  },
];

function TurnBlockedBanner() {
  const blocked = useSettingsSaveBlocked();
  if (!blocked) return null;
  return (
    <div className="border-b border-(--_dk-amber-900\/20) bg-(--_dk-amber-900\/20) px-4 py-2 text-sm text-(--_dk-amber-500)">
      {i18n.t("settings.dialog.busy")}
    </div>
  );
}

function SectionContent({ section }: { section: SettingsSection }) {
  switch (section) {
    case "connection":
      return <ProvidersSection />;
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
  const loadError = useSettingsStore((s) => s.loadError);
  const showSkeleton = useSettingsStore((s) =>
    sectionNeedsSkeleton(s.section, s),
  );
  const closeSettings = useSettingsStore((s) => s.closeSettings);
  const setSection = useSettingsStore((s) => s.setSection);

  return (
    <FloatingDialog
      visible={open}
      title={i18n.t("settings.dialog.revision", { revision })}
      onClose={() => void closeSettings()}
      defaultWidth={860}
      defaultHeight={640}
    >
      <div className="flex h-full flex-col">
        <TurnBlockedBanner />

        <div className="flex min-h-0 flex-1">
          <nav className="settings-nav shrink-0 bg-(--_dk-side)">
            {NAV_GROUPS.map((group) => (
              <div key={group.titleKey} className="settings-nav-group">
                <div className="settings-nav-group-title">{i18n.t(group.titleKey)}</div>
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
                        <span>{i18n.t(item.labelKey)}</span>
                      </span>
                    </button>
                  );
                })}
              </div>
            ))}
          </nav>

          <div className="min-w-0 flex-1 bg-(--_dk-overlay)">
            <div key={section} className="settings-content-enter h-full">
              {showSkeleton ? (
                <div className="h-full overflow-y-auto px-6 py-5">
                  <SettingsSkeleton />
                </div>
              ) : loadError ? (
                <p className="px-6 py-5 text-sm text-(--_dk-red-500)">
                  {loadError}
                </p>
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
