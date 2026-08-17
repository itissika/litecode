import type { ReactNode } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { Logo } from "../../components/Logo";
import { useServerVersionTags, VersionTags } from "../../components/VersionTags";
import { ABOUT } from "../../lib/about";

const RUNTIME_DEPS = [
  { name: "React", version: "19.1.0" },
  { name: "dockview-react", version: "7.0.2" },
  { name: "monaco-editor", version: "0.55.1" },
] as const;

function AboutLink({
  href,
  children,
}: {
  href: string;
  children: ReactNode;
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-dk-xs text-(--_dk-accent-hover) underline underline-offset-2 hover:text-(--_dk-text-primary)"
    >
      {children}
    </a>
  );
}

function MetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 text-dk-xs">
      <span className="shrink-0 text-(--_dk-text-muted)">{label}</span>
      <span className="min-w-0 text-right text-(--_dk-text-secondary)">{children}</span>
    </div>
  );
}

export function AboutContent({ replay = 0 }: { replay?: number }) {
  const versionTags = useServerVersionTags();

  return (
    <div className="flex h-full flex-col items-center gap-5 px-6 py-6 text-(--_dk-text-muted)">
      <div className="flex flex-col items-center gap-3 text-center">
        <Logo size="md" replay={replay} />
        <VersionTags {...versionTags} size="sm" github />
        <p className="max-w-sm text-dk-xs leading-relaxed text-(--_dk-text-muted)">
          {ABOUT.tagline}
        </p>
      </div>

      <div className="w-full max-w-sm space-y-2">
        <MetaRow label="License">
          <AboutLink href={ABOUT.licenseUrl}>{ABOUT.license}</AboutLink>
        </MetaRow>
        <MetaRow label="Releases">
          <AboutLink href={ABOUT.releasesUrl}>Latest release</AboutLink>
        </MetaRow>
      </div>

      <p className="text-center text-dk-2xs text-(--_dk-text-disabled)">{ABOUT.copyright}</p>

      <div className="mt-auto w-full max-w-sm border-t border-(--_dk-line) pt-4">
        <p className="mb-2 text-center text-dk-2xs uppercase tracking-wide text-(--_dk-text-disabled)">
          Built with
        </p>
        <div className="space-y-1">
          {RUNTIME_DEPS.map((dep) => (
            <p key={dep.name} className="text-center font-mono text-dk-2xs text-(--_dk-text-disabled)">
              {dep.name} v{dep.version}
            </p>
          ))}
        </div>
      </div>
    </div>
  );
}

export function AboutPanel(_props: IDockviewPanelProps) {
  return <AboutContent />;
}
