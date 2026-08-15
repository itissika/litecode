/** Parse `user@host` or `user@host:port` (IPv6 hosts must use bracket form later if needed). */
export function parseUserAtHost(input: string): { user?: string; host: string; port?: number } {
  const trimmed = input.trim();
  if (!trimmed) throw new Error("Enter user@host (for example user@192.168.1.10).");
  if (/\s/.test(trimmed)) throw new Error("user@host must not contain spaces.");

  let user: string | undefined;
  let rest = trimmed;
  const at = trimmed.lastIndexOf("@");
  if (at >= 0) {
    user = trimmed.slice(0, at).trim();
    rest = trimmed.slice(at + 1).trim();
    if (!user) throw new Error("SSH user is required before @.");
    if (!rest) throw new Error("SSH host is required after @.");
  }

  let host = rest;
  let port: number | undefined;
  // host:port — avoid splitting bare IPv6; only split when a single trailing :digits
  const portMatch = /^(.+):(\d{1,5})$/.exec(rest);
  if (portMatch && !rest.includes("::")) {
    host = portMatch[1]!;
    const parsed = Number(portMatch[2]);
    if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
      throw new Error("SSH port must be between 1 and 65535.");
    }
    port = parsed;
  }

  return {
    host,
    ...(user ? { user } : {}),
    ...(port ? { port } : {}),
  };
}

export function formatUserAtHost(target: { user?: string; host: string; port?: number }): string {
  const base = target.user ? `${target.user}@${target.host}` : target.host;
  return target.port ? `${base}:${target.port}` : base;
}
