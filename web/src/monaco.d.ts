// Type declarations for the deep ESM Monaco imports used in lib/monaco.ts.
// monaco-editor only ships types at its package root; the basic-languages
// contributions and the JSON language service are side-effect imports that
// carry no .d.ts of their own. `editor.api` is typed as the full monaco API
// (its runtime surface is a subset of it — no language services).

declare module "monaco-editor/esm/vs/editor/editor.api" {
  export * from "monaco-editor";
}

declare module "monaco-editor/esm/vs/basic-languages/*" {}

declare module "monaco-editor/esm/vs/language/json/monaco.contribution" {}
