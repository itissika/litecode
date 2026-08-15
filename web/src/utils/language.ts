export function languageFromPath(path: string): string {
  const dot = path.lastIndexOf(".");
  if (dot < 0) return "plaintext";

  const ext = path.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    rs: "rust",
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    mjs: "javascript",
    cjs: "javascript",
    json: "json",
    md: "markdown",
    mdx: "markdown",
    py: "python",
    toml: "ini",
    yaml: "yaml",
    yml: "yaml",
    css: "css",
    scss: "scss",
    html: "html",
    htm: "html",
    xml: "xml",
    sql: "sql",
    sh: "shell",
    bash: "shell",
    zsh: "shell",
    go: "go",
    java: "java",
    c: "c",
    h: "c",
    cpp: "cpp",
    hpp: "cpp",
    cs: "csharp",
    rb: "ruby",
    php: "php",
    swift: "swift",
    kt: "kotlin",
    lua: "lua",
    dockerfile: "dockerfile",
  };

  return map[ext] ?? "plaintext";
}

export function fileNameFromPath(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash >= 0 ? path.slice(slash + 1) : path;
}
