import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { knowledgeRailBadge } from "../../lib/knowledge/validate";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { getPanelIcon } from "./icons";

function badgeLabel(tone: "error" | "warning", count: number): string {
  return tone === "error"
    ? `${count} 个节点有校验错误`
    : `${count} 个节点有过时或损坏风险`;
}

export function EdgeTab(props: IDockviewPanelProps) {
  const title = props.api.title ?? props.api.id;
  const Icon = getPanelIcon(props.api.component);
  const isKnowledge = props.api.component === "knowledge";
  const issues = useKnowledgeStore((state) => state.issues);
  const loadKnowledge = useKnowledgeStore((state) => state.load);
  const badge = isKnowledge ? knowledgeRailBadge(issues) : null;

  useEffect(() => {
    if (!isKnowledge) return;
    void loadKnowledge();
  }, [isKnowledge, loadKnowledge]);

  const groupApi = props.api.group.api;
  const [isCollapsed, setIsCollapsed] = useState(groupApi.isCollapsed());

  // dockview does NOT re-render the tab component when a panel moves between
  // edges, so a `location`-based layout computed at render time would stay
  // stale. That leaves the bottom layout (icon + English title) rendering
  // inside the side edge's vertical tab strip (writing-mode: vertical-rl) →
  // "vertical English" text. Track it in state and update on location change.
  const [isBottom, setIsBottom] = useState(
    () =>
      props.api.location.type === "edge" &&
      props.api.location.position === "bottom",
  );

  useEffect(() => {
    const d1 = groupApi.onDidCollapsedChange((e) =>
      setIsCollapsed(e.isCollapsed),
    );
    const d2 = props.api.onDidLocationChange(() => {
      const loc = props.api.location;
      setIsBottom(loc.type === "edge" && loc.position === "bottom");
    });
    return () => {
      d1.dispose();
      d2.dispose();
    };
  }, [groupApi, props.api]);

  const isExpanded = !isCollapsed;
  const glow = isExpanded ? "0 0 8px rgba(41, 151, 255, 0.15)" : "none";
  const tip = badge ? `${title} · ${badgeLabel(badge.tone, badge.count)}` : title;
  const mark = badge ? (
    <span
      className={`knowledge-rail-badge is-${badge.tone}`}
      aria-label={badgeLabel(badge.tone, badge.count)}
    >
      {badge.count > 99 ? "99+" : badge.count}
    </span>
  ) : null;

  if (isBottom) {
    return (
      <div className="flex items-center gap-1.5 px-1.5 h-full transition-colors duration-120 hover:brightness-125 active:brightness-75">
        <span className="knowledge-rail-icon flex-shrink-0">
          <span className="transition-all duration-120" style={{ filter: glow }}>
            <Icon
              size={14}
              weight={isExpanded ? ("fill" as "regular") : "regular"}
            />
          </span>
          {mark}
        </span>
        <span className="text-xs truncate select-none">{title}</span>
      </div>
    );
  }

  return (
    <div
      className="flex items-center justify-center h-full w-full transition-colors duration-120 hover:brightness-125 active:brightness-75"
      title={tip}
    >
      <span className="knowledge-rail-icon">
        <span className="transition-all duration-120" style={{ filter: glow }}>
          <Icon
            size={16}
            weight={isExpanded ? ("fill" as "regular") : "regular"}
          />
        </span>
        {mark}
      </span>
    </div>
  );
}
