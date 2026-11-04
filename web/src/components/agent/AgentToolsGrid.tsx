import { useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ListChecks } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import {
  isCodeSurface,
  isConfigurableTool,
  type AgentProfile,
  type AgentToolBinding,
  type AvailableTool,
} from "../../api/settings";
import { FoldCard } from "../FoldCard";
import { Dropdown } from "../ui/Dropdown";
import { groupToolCards } from "../../dockview/panels/settings/toolGroups";

/** One MCP server as the tool grid needs it: its id and discovered tools. */
export interface AgentToolServer {
  id: string;
  tools?: { name: string; description: string }[];
}

export function bindingFor(
  tools: Record<string, AgentToolBinding>,
  toolId: string,
): AgentToolBinding {
  return tools[toolId] ?? { enabled: false, preset: "ALL" };
}

function McpToolVisibilityControl({
  serverId,
  binding,
  tools,
  disabled,
  onChange,
}: {
  serverId: string;
  binding: AgentToolBinding;
  tools: { name: string; description: string }[];
  disabled: boolean;
  onChange: (allowed_tools: string[]) => void;
}) {
  const { t } = useTranslation();
  const selected =
    binding.allowed_tools == null
      ? new Set(tools.map((tool) => tool.name))
      : new Set(binding.allowed_tools);

  const toggleTool = (name: string) => {
    const next = new Set(selected);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onChange(
      tools.filter((tool) => next.has(tool.name)).map((tool) => tool.name),
    );
  };

  // Virtualize the tool list: MCP servers can expose hundreds of tools, so
  // only the visible rows are mounted (same tanstack virtualizer as MessageList).
  // The scroll element lives inside the Dropdown's portal and only mounts once
  // the panel opens, so it's stored via a callback ref → state: that forces a
  // re-render when it mounts, letting the virtualizer attach its observers.
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: tools.length,
    getScrollElement: () => scrollEl,
    estimateSize: () => 32,
    overscan: 6,
  });

  return (
    <Dropdown
      variant="panel"
      align="right"
      flip
      closeOnSelect={false}
      panelClassName="w-[360px] max-w-[calc(100vw-16px)] p-2"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          aria-label={t("settings.agents.mcpVisibilityAria", { serverId })}
          aria-expanded={open}
          className={[
            disabled ? "" : "tool-binding-action",
            "btn btn-ghost btn-xs",
          ]
            .filter(Boolean)
            .join(" ")}
          onClick={(event) => {
            event.stopPropagation();
            toggle();
          }}
        >
          <ListChecks size={14} />
          {t("common.tools")}
        </button>
      )}
    >
      <div className="space-y-1">
        <p className="px-1 pb-1 text-xs text-(--_dk-text-muted)">
          {t("settings.agents.visibleToModel")}
        </p>
        {tools.length === 0 ? (
          <p className="px-1 py-2 text-xs text-(--_dk-text-disabled)">
            {t("settings.agents.noToolsListed")}
          </p>
        ) : (
          <div ref={setScrollEl} className="max-h-64 overflow-y-auto">
            <div
              style={{
                height: `${virtualizer.getTotalSize()}px`,
                position: "relative",
                width: "100%",
              }}
            >
              {virtualizer.getVirtualItems().map((vi) => {
                const tool = tools[vi.index];
                const checked = selected.has(tool.name);
                return (
                  <div
                    key={vi.key}
                    data-index={vi.index}
                    ref={virtualizer.measureElement}
                    className="py-1"
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${vi.start}px)`,
                    }}
                  >
                    <FoldCard
                      label={
                        <label
                          className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                          onClick={(event) => event.stopPropagation()}
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            disabled={disabled}
                            onChange={() => toggleTool(tool.name)}
                            className="accent-(--_dk-accent-hover)"
                          />
                          <span className="min-w-0 truncate font-mono text-xs text-(--_dk-text-primary)">
                            {tool.name}
                          </span>
                        </label>
                      }
                      headerAriaLabel={t("settings.agents.mcpToolDetailsAria", { name: tool.name })}
                      className="border-b border-(--_dk-line)"
                      contentClassName="px-5 pb-2 text-xs text-(--_dk-text-secondary)"
                    >
                      {tool.description ||
                        t("settings.agents.noMcpDescription")}
                    </FoldCard>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </Dropdown>
  );
}

/** Capability card grid shared by Settings → Agents and the bot editor. */
export function AgentToolsGrid({
  draft,
  bindableTools,
  mcpServers,
  saveBlocked,
  onBindingChange,
  gridStyle,
  note,
}: {
  draft: AgentProfile;
  bindableTools: AvailableTool[];
  mcpServers: AgentToolServer[];
  saveBlocked: boolean;
  onBindingChange: (toolId: string, patch: Partial<AgentToolBinding>) => void;
  /** Inline style for the card grid (e.g. force columns below sm breakpoint). */
  gridStyle?: CSSProperties;
  /** Extra guidance rendered under the section title (e.g. subagent deny semantics). */
  note?: ReactNode;
}) {
  const { t } = useTranslation();
  /** Sections, each with a header row, flattened so the grid stays one map. */
  const rows = useMemo(
    () =>
      groupToolCards(bindableTools).flatMap((group) => [
        { kind: "head" as const, group },
        ...group.tools.map((entry) => ({ kind: "card" as const, entry })),
      ]),
    [bindableTools],
  );
  if (bindableTools.length === 0) {
    return (
      <div className="space-y-2">
        <h3 className="settings-section-title">{t("settings.agents.toolBindings")}</h3>
        {note ? (
          <p className="text-xs text-(--_dk-text-muted)">{note}</p>
        ) : null}
        <p className="text-sm text-(--_dk-amber-500)">
          {t("settings.agents.noBindableTools")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <h3 className="settings-section-title">{t("settings.agents.toolBindings")}</h3>
      {note ? <p className="text-xs text-(--_dk-text-muted)">{note}</p> : null}
      <div
        className="grid grid-cols-1 gap-3 p-3 sm:grid-cols-2 xl:grid-cols-3"
        style={gridStyle}
        role="group"
        aria-label={t("settings.agents.agentToolBindings")}
      >
        {rows.map((row) => {
          if (row.kind === "head") {
            const { group } = row;
            const on = group.tools.filter(
              (tool) => bindingFor(draft.tools, tool.id).enabled,
            ).length;
            return (
              <div
                key={`head-${group.kind}`}
                role="presentation"
                className="col-span-full flex items-baseline justify-between gap-2 pt-2 first:pt-0"
              >
                <h4 className="text-dk-xs font-medium text-(--_dk-text-muted)">
                  {t(`settings.agents.toolGroup.${group.kind}`)}
                </h4>
                <span className="settings-header-meta">
                  {t("settings.agents.toolGroupCount", {
                    enabled: on,
                    total: group.tools.length,
                  })}
                </span>
              </div>
            );
          }
          const entry = row.entry;
          const binding = bindingFor(draft.tools, entry.id);
          const configurable = isConfigurableTool(entry);
          const enabled = binding.enabled;
          const preset = binding.preset ?? "ALL";
          const serverId = entry.id.slice("mcp_".length);
          const mcpTools =
            entry.kind === "mcp"
              ? (mcpServers.find((server) => server.id === serverId)?.tools ??
                [])
              : [];

          const toggleEnabled = () => {
            if (saveBlocked) return;
            onBindingChange(entry.id, { enabled: !enabled });
          };
          const memberLine =
            entry.members && entry.members.length > 1
              ? `${entry.kind} · ${entry.members.join(", ")}`
              : entry.kind;
          const seriesDetail = entry.overridden
            ? `${memberLine} · ${t("settings.agents.workspaceOverride")}`
            : memberLine;
          const status = entry.status && entry.status !== "ready" ? entry.status : null;
          const statusTone =
            status === "failed" || status === "down"
              ? "tag-danger"
              : status === "warming"
                ? "tag-warn"
                : "tag-neutral";

          return (
            <div
              key={entry.id}
              data-enabled={enabled ? "true" : "false"}
              className="tool-binding-card flex flex-col overflow-hidden"
            >
              <button
                type="button"
                disabled={saveBlocked}
                aria-pressed={enabled}
                aria-label={t("settings.agents.toolBindingAria", { id: entry.id, state: enabled ? t("settings.agents.enabled") : t("settings.agents.disabled") })}
                className="tool-binding-toggle"
                onClick={toggleEnabled}
              />
              <div className="tool-binding-content flex flex-col">
                <div className="flex w-full items-start justify-between gap-2 p-3">
                  <div className="min-w-0">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <p className="tool-binding-title truncate font-mono text-sm text-(--_dk-text-primary)">
                        {entry.label || entry.id}
                      </p>
                      {isCodeSurface(entry) ? (
                        <span className="tag tag-neutral tag-xs tag-outline shrink-0">
                          code mode
                        </span>
                      ) : null}
                    </div>
                    <p
                      className="mt-0.5 truncate text-dk-xs text-(--_dk-text-disabled)"
                      title={seriesDetail}
                    >
                      {seriesDetail}
                    </p>
                  </div>
                  <div className="flex shrink-0 flex-col items-end gap-1">
                    <span
                      className={`tag ${enabled ? "tag-ok" : "tag-neutral"} tag-sm tag-outline`}
                    >
                      {enabled ? t("common.on") : t("common.off")}
                    </span>
                    {status ? (
                      <span className={`tag ${statusTone} tag-xs tag-outline`}>{status}</span>
                    ) : null}
                  </div>
                </div>

                <div className="tool-binding-foot px-3 py-2.5">
                  {configurable ? (
                    <div
                      className="flex flex-wrap gap-1.5"
                      role="group"
                      aria-label={t("settings.agents.presetAria", { id: entry.id })}
                    >
                      {(["ALL", "SAFE"] as const).map((value) => (
                        <button
                          key={value}
                          type="button"
                          disabled={saveBlocked || !enabled}
                          aria-pressed={preset === value}
                          onClick={(event) => {
                            event.stopPropagation();
                            onBindingChange(entry.id, {
                              preset: value,
                            });
                          }}
                          className={[
                            // Only intercept hits when the control is live; disabled
                            // buttons stay click-through so the full-card toggle works.
                            enabled && !saveBlocked
                              ? "tool-binding-action"
                              : "",
                            preset === value
                              ? "btn-primary btn-xs"
                              : "btn-ghost btn-xs",
                          ]
                            .filter(Boolean)
                            .join(" ")}
                        >
                          {value}
                        </button>
                      ))}
                    </div>
                  ) : (
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-xs text-(--_dk-text-disabled)">
                        {entry.kind === "mcp"
                          ? t("settings.agents.noPreset")
                          : t("settings.agents.notConfigurable")}
                      </p>
                      {entry.kind === "mcp" ? (
                        <McpToolVisibilityControl
                          serverId={serverId}
                          binding={binding}
                          tools={mcpTools}
                          disabled={saveBlocked || !enabled}
                          onChange={(allowed_tools) =>
                            onBindingChange(entry.id, { allowed_tools })
                          }
                        />
                      ) : null}
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
