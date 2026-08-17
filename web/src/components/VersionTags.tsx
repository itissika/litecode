import type { ReactNode } from "react";
import { GithubLogo } from "@phosphor-icons/react";

import {
  ABOUT,
  formatServerVersion,
  parseVersionChannel,
  shouldShowVersionChannel,
  versionChannelLabel,
  versionChannelTagTone,
  type VersionChannel,
} from "../lib/about";
import { useConnectionStore } from "../stores/connectionStore";

type VersionTagsProps = {
  channelRaw?: string;
  version?: string;
  waiting?: boolean;
  size?: "xs" | "sm";
  github?: boolean;
  className?: string;
};

function versionLabel(version: string | undefined, waiting: boolean): string {
  if (version) return formatServerVersion(version);
  return waiting ? "…" : "—";
}

function channelToneClass(channel: VersionChannel): string {
  switch (versionChannelTagTone(channel)) {
    case "warn":
      return "text-(--_dk-amber-500)";
    case "ok":
      return "text-(--_dk-emerald-500)";
    default:
      return "text-(--_dk-text-muted)";
  }
}

const VERSION_TAG_BASE =
  "inline-flex items-center font-normal leading-none tracking-wide whitespace-nowrap select-none";

export function VersionTags({
  channelRaw,
  version,
  waiting = false,
  size = "sm",
  github = false,
  className = "",
}: VersionTagsProps) {
  const channel = parseVersionChannel(channelRaw);
  const textSize = size === "xs" ? "text-dk-2xs" : "text-dk-xs";
  const showChannel = shouldShowVersionChannel(channel);

  return (
    <div className={`flex items-center gap-1.5 ${className}`.trim()}>
      {showChannel && channel ? (
        <ChannelTag
          channel={channel}
          textSize={textSize}
          waiting={waiting && !channelRaw}
        />
      ) : null}
      <span
        className={`${VERSION_TAG_BASE} ${textSize} font-mono tabular-nums text-(--_dk-text-muted)`}
      >
        {versionLabel(version, waiting)}
      </span>
      {github ? (
        <a
          href={ABOUT.repositoryUrl}
          target="_blank"
          rel="noreferrer"
          className="btn-ghost btn-icon btn-xs text-(--_dk-text-muted) hover:text-(--_dk-text-primary)"
          aria-label="Open source repository on GitHub"
          title="GitHub repository"
        >
          <GithubLogo size={16} weight="fill" />
        </a>
      ) : null}
    </div>
  );
}

function ChannelTag({
  channel,
  textSize,
  waiting,
}: {
  channel: VersionChannel;
  textSize: string;
  waiting: boolean;
}) {
  if (waiting) {
    return (
      <span className={`${VERSION_TAG_BASE} ${textSize} uppercase text-(--_dk-text-muted)`}>
        …
      </span>
    );
  }

  return (
    <span
      className={`${VERSION_TAG_BASE} ${textSize} uppercase ${channelToneClass(channel)}`}
    >
      {versionChannelLabel(channel)}
    </span>
  );
}

export function useServerVersionTags() {
  const serverVersion = useConnectionStore((s) => s.serverVersion);
  const serverVersionChannel = useConnectionStore((s) => s.serverVersionChannel);
  const connectionState = useConnectionStore((s) => s.state);
  const waiting = connectionState !== "connected" && !serverVersion;

  return {
    channelRaw: serverVersionChannel,
    version: serverVersion,
    waiting,
  };
}
