/** Build handshake URL with `?token=` for remote/local READY pages. */
export function handshakeUrl(baseUrl: string, token: string): string {
  const trimmed = baseUrl.trim();
  const withSlash = trimmed.endsWith("/") ? trimmed : `${trimmed}/`;
  const url = new URL(withSlash);
  url.searchParams.set("token", token);
  return url.toString();
}
