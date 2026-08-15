import type { IDockviewPanelProps } from "dockview-react";
import { Logo } from "../../components/Logo";

export function AboutContent({ replay = 0 }: { replay?: number }) {
  return (
    <div
      className="flex flex-col items-center justify-center h-full gap-4"
      style={{ color: "var(--_dk-text-muted)" }}
    >
      <Logo size="md" replay={replay} />
      <div className="flex flex-col items-center gap-2">
        <p className="text-xs">React v19.1.0</p>
        <p className="text-xs">dockview-react v7.0.2</p>
        <p className="text-xs">monaco-editor v0.55.1</p>
      </div>
    </div>
  );
}

export function AboutPanel(_props: IDockviewPanelProps) {
  return <AboutContent />;
}
