/** One glob per line; `#` comments and blanks are dropped. */
export function globsFromText(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#") || seen.has(line)) continue;
    seen.add(line);
    out.push(line);
  }
  return out;
}

export function textFromGlobs(globs: string[]): string {
  return globs.join("\n");
}
