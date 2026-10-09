import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor/editor/editor.api";

// Local Monaco bundle: no CDN dependency at runtime.
//
// `editor.api` is the editor API only. Since 0.56 the hover widget, the
// completion widget, and the rest of the editor contributions are separate
// side-effect imports. `register.all` covers most of them; the completion
// dropdown, document semantic tokens, and go-to commands are still wired
// only by the full bundle, so they are imported here. Syntax highlighting
// comes from the language definitions below. The TypeScript/JS/CSS/HTML
// language services stay out — hover, completion, and diagnostics come from
// the workspace LSP, and those services would only add workers that disagree
// with it.
import "monaco-editor/features/register.all";
import "monaco-editor/editor/browser/coreCommands";
import "monaco-editor/editor/contrib/caretOperations/browser/caretOperations";
import "monaco-editor/editor/contrib/dropOrPasteInto/browser/copyPasteContribution";
import "monaco-editor/editor/contrib/gotoSymbol/browser/goToCommands";
import "monaco-editor/editor/contrib/gotoError/browser/markerSelectionStatus";
import "monaco-editor/editor/contrib/semanticTokens/browser/documentSemanticTokens";
import "monaco-editor/editor/contrib/suggest/browser/suggestController";
import "monaco-editor/editor/common/standaloneStrings";

// Basic languages (c/cpp are both registered by the cpp contribution).
// Monaco 0.57 moved each language to languages/definitions/<id>/register.
import "monaco-editor/languages/definitions/rust/register";
import "monaco-editor/languages/definitions/typescript/register";
import "monaco-editor/languages/definitions/javascript/register";
import "monaco-editor/languages/definitions/markdown/register";
import "monaco-editor/languages/definitions/python/register";
import "monaco-editor/languages/definitions/ini/register";
import "monaco-editor/languages/definitions/yaml/register";
import "monaco-editor/languages/definitions/css/register";
import "monaco-editor/languages/definitions/scss/register";
import "monaco-editor/languages/definitions/html/register";
import "monaco-editor/languages/definitions/xml/register";
import "monaco-editor/languages/definitions/sql/register";
import "monaco-editor/languages/definitions/shell/register";
import "monaco-editor/languages/definitions/go/register";
import "monaco-editor/languages/definitions/java/register";
import "monaco-editor/languages/definitions/cpp/register";
import "monaco-editor/languages/definitions/csharp/register";
import "monaco-editor/languages/definitions/ruby/register";
import "monaco-editor/languages/definitions/php/register";
import "monaco-editor/languages/definitions/swift/register";
import "monaco-editor/languages/definitions/kotlin/register";
import "monaco-editor/languages/definitions/lua/register";
import "monaco-editor/languages/definitions/dockerfile/register";
import "monaco-editor/languages/definitions/wgsl/register";
import "monaco-editor/languages/definitions/powershell/register";
import "monaco-editor/languages/definitions/bat/register";
import "monaco-editor/languages/definitions/graphql/register";
import "monaco-editor/languages/definitions/protobuf/register";
import "monaco-editor/languages/definitions/perl/register";
import "monaco-editor/languages/definitions/r/register";
import "monaco-editor/languages/definitions/scala/register";
import "monaco-editor/languages/definitions/dart/register";
import "monaco-editor/languages/definitions/elixir/register";
import "monaco-editor/languages/definitions/clojure/register";
import "monaco-editor/languages/definitions/less/register";
import "monaco-editor/languages/definitions/hcl/register";
import "monaco-editor/languages/definitions/objective-c/register";
import "monaco-editor/languages/definitions/systemverilog/register";
import "monaco-editor/languages/definitions/vb/register";
import "monaco-editor/languages/definitions/fsharp/register";
import "monaco-editor/languages/definitions/solidity/register";
import "monaco-editor/language/json/monaco.contribution";
import { registerShaderSupport } from "./monacoShaders";

registerShaderSupport(monaco);

// Workers: the editor worker covers tokenization for every basic language; the
// JSON language service needs its own worker.
import editorWorker from "monaco-editor/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/language/json/json.worker?worker";

self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string): Worker {
    if (label === "json") return new jsonWorker();
    return new editorWorker();
  },
};

// Use the bundled instance instead of the default jsdelivr CDN loader.
loader.config({ monaco });

export default monaco;
