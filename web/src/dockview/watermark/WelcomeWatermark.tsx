import type { IWatermarkPanelProps } from "dockview-react";
import { Logo } from "../../components/Logo";

export function WelcomeWatermark(_props: IWatermarkPanelProps) {
  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-6"
      style={{ background: "var(--_dk-root)" }}
    >
      <Logo size="md" animated={false} style={{ opacity: 0.35 }} />
      <div className="flex flex-col items-center gap-1 text-xs" style={{ color: "var(--_dk-text-disabled)" }}>
        <p>Open a folder to start</p>
      </div>
    </div>
  );
}
