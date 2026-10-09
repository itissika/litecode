// Type declarations for the Monaco subpath imports used in lib/monaco.ts.
// 0.57's export map is `"./*" -> "./esm/vs/*.js"`, so public paths no longer
// include the `esm/vs` prefix.
// monaco-editor only ships types at its package root; the feature registers,
// language contributions, and JSON language service are side-effect imports
// that carry no .d.ts of their own. `editor.api` is typed as the full monaco
// API (its runtime surface is a subset of it — no language services).

declare module "monaco-editor/editor/editor.api" {
  export * from "monaco-editor";
}

declare module "monaco-editor/features/register.all" {}
declare module "monaco-editor/editor/browser/coreCommands" {}
declare module "monaco-editor/editor/contrib/caretOperations/browser/caretOperations" {}
declare module "monaco-editor/editor/contrib/dropOrPasteInto/browser/copyPasteContribution" {}
declare module "monaco-editor/editor/contrib/gotoSymbol/browser/goToCommands" {}
declare module "monaco-editor/editor/contrib/gotoError/browser/markerSelectionStatus" {}
declare module "monaco-editor/editor/contrib/semanticTokens/browser/documentSemanticTokens" {}
declare module "monaco-editor/editor/contrib/suggest/browser/suggestController" {}
declare module "monaco-editor/editor/common/standaloneStrings" {}

declare module "monaco-editor/languages/definitions/*/register" {}

declare module "monaco-editor/language/json/monaco.contribution" {}
