import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import astro from "shiki/langs/astro.mjs";
import bash from "shiki/langs/bash.mjs";
import c from "shiki/langs/c.mjs";
import cpp from "shiki/langs/cpp.mjs";
import csharp from "shiki/langs/csharp.mjs";
import css from "shiki/langs/css.mjs";
import diff from "shiki/langs/diff.mjs";
import docker from "shiki/langs/docker.mjs";
import dotenv from "shiki/langs/dotenv.mjs";
import elixir from "shiki/langs/elixir.mjs";
import go from "shiki/langs/go.mjs";
import graphql from "shiki/langs/graphql.mjs";
import html from "shiki/langs/html.mjs";
import ini from "shiki/langs/ini.mjs";
import java from "shiki/langs/java.mjs";
import javascript from "shiki/langs/javascript.mjs";
import json from "shiki/langs/json.mjs";
import jsonc from "shiki/langs/jsonc.mjs";
import jsx from "shiki/langs/jsx.mjs";
import kotlin from "shiki/langs/kotlin.mjs";
import lua from "shiki/langs/lua.mjs";
import makefile from "shiki/langs/makefile.mjs";
import markdown from "shiki/langs/markdown.mjs";
import php from "shiki/langs/php.mjs";
import python from "shiki/langs/python.mjs";
import ruby from "shiki/langs/ruby.mjs";
import rust from "shiki/langs/rust.mjs";
import scala from "shiki/langs/scala.mjs";
import shell from "shiki/langs/shell.mjs";
import sql from "shiki/langs/sql.mjs";
import svelte from "shiki/langs/svelte.mjs";
import swift from "shiki/langs/swift.mjs";
import toml from "shiki/langs/toml.mjs";
import tsx from "shiki/langs/tsx.mjs";
import typescript from "shiki/langs/typescript.mjs";
import vue from "shiki/langs/vue.mjs";
import xml from "shiki/langs/xml.mjs";
import yaml from "shiki/langs/yaml.mjs";
import zig from "shiki/langs/zig.mjs";
import darkPlus from "shiki/themes/dark-plus.mjs";
import minLight from "shiki/themes/min-light.mjs";

export { SHIKI_THEME_DARK, SHIKI_THEME_LIGHT } from "../theme/shiki";

const LANG_ALIASES: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  sh: "bash",
  zsh: "bash",
  shell: "bash",
  yml: "yaml",
  md: "markdown",
  mdx: "markdown",
  rs: "rust",
  py: "python",
  rb: "ruby",
  cs: "csharp",
  "c#": "csharp",
  "c++": "cpp",
  cc: "cpp",
  cxx: "cpp",
  h: "c",
  hpp: "cpp",
  kt: "kotlin",
  kts: "kotlin",
  golang: "go",
  dockerfile: "docker",
  gql: "graphql",
  env: "dotenv",
  vuejs: "vue",
  sveltejs: "svelte",
  prisma: "graphql",
};

export function normalizeLang(raw: string): string {
  const lang = raw.toLowerCase();
  return LANG_ALIASES[lang] ?? lang;
}

const SUPPORTED_LANGS = new Set([
  "astro",
  "bash",
  "c",
  "cpp",
  "csharp",
  "css",
  "diff",
  "docker",
  "dotenv",
  "elixir",
  "go",
  "graphql",
  "html",
  "ini",
  "java",
  "javascript",
  "json",
  "jsonc",
  "jsx",
  "kotlin",
  "lua",
  "makefile",
  "markdown",
  "php",
  "python",
  "ruby",
  "rust",
  "scala",
  "shell",
  "sql",
  "svelte",
  "swift",
  "toml",
  "tsx",
  "typescript",
  "vue",
  "xml",
  "yaml",
  "zig",
]);

export function isSupportedHighlightLang(lang: string): boolean {
  return SUPPORTED_LANGS.has(normalizeLang(lang));
}

let highlighterPromise: Promise<HighlighterCore> | null = null;

export function getMarkdownHighlighter() {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighterCore({
      themes: [darkPlus, minLight],
      langs: [
        astro,
        bash,
        c,
        cpp,
        csharp,
        css,
        diff,
        docker,
        dotenv,
        elixir,
        go,
        graphql,
        html,
        ini,
        java,
        javascript,
        json,
        jsonc,
        jsx,
        kotlin,
        lua,
        makefile,
        markdown,
        php,
        python,
        ruby,
        rust,
        scala,
        shell,
        sql,
        svelte,
        swift,
        toml,
        tsx,
        typescript,
        vue,
        xml,
        yaml,
        zig,
      ],
      engine: createJavaScriptRegexEngine(),
    });
  }
  return highlighterPromise;
}
