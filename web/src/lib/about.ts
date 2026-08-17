export type VersionChannel = "dev" | "nightly" | "official";

export const ABOUT = {
  tagline: "A coding agent framework obsessively optimized for runtime lightness",
  repositoryUrl: "https://github.com/itissika/litecode",
  releasesUrl: "https://github.com/itissika/litecode/releases/latest",
  license: "MIT",
  licenseUrl: "https://github.com/itissika/litecode/blob/main/LICENSE",
  copyright: "Copyright © 2025 LiteCode contributors",
} as const;

export function parseVersionChannel(raw: string | undefined): VersionChannel | null {
  switch (raw) {
    case "dev":
    case "nightly":
    case "official":
      return raw;
    default:
      return null;
  }
}

export function formatServerVersion(raw: string): string {
  return raw.startsWith("v") ? raw : `v${raw}`;
}

/** Official is internal-only: UI shows version without a channel tag. */
export function shouldShowVersionChannel(channel: VersionChannel | null): boolean {
  return channel !== null && channel !== "official";
}

export function versionChannelLabel(channel: VersionChannel): string {
  switch (channel) {
    case "dev":
      return "Dev";
    case "nightly":
      return "Nightly";
    case "official":
      return "Official";
  }
}

export function versionChannelTagTone(
  channel: VersionChannel,
): "neutral" | "warn" | "ok" {
  switch (channel) {
    case "dev":
      return "neutral";
    case "nightly":
      return "warn";
    case "official":
      return "ok";
  }
}
