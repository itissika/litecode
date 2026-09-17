import {
  datatypes,
  intrinsicfunctions,
  keywords,
  preprocessors,
  semantics,
  semanticsNum,
  type IEntry,
} from "./hlslGlobals";
import {
  pragmaDirectives,
  shaderLabKeywords,
  shaderLabPropertyTypes,
  unityBuiltinFunctions,
  unityBuiltinMacros,
  unityBuiltinVariables,
} from "./unityGlobals";
import {
  urpBuiltinFunctions,
  urpBuiltinMacros,
  urpBuiltinVariables,
  urpIncludePaths,
} from "./urpGlobals";

export type ShaderSuggestKind =
  | "keyword"
  | "function"
  | "type"
  | "variable"
  | "constant"
  | "file"
  | "semantic";

export type ShaderSuggest = {
  label: string;
  kind: ShaderSuggestKind;
  detail?: string;
  documentation?: string;
  insertText?: string;
  snippet?: boolean;
};

let cached: ShaderSuggest[] | null = null;

function hlslDetail(name: string, entry: IEntry, kind: string): string | undefined {
  if (!entry.parameters?.length) return kind;
  const params = entry.parameters.map((p) => p.label).join(", ");
  return `(${kind}) ${name}(${params})`;
}

export function shaderSuggests(): ShaderSuggest[] {
  if (cached) return cached;

  const out: ShaderSuggest[] = [];
  const seen = new Set<string>();
  const add = (item: ShaderSuggest) => {
    if (seen.has(item.label)) return;
    seen.add(item.label);
    out.push(item);
  };

  for (const [name, entry] of Object.entries(datatypes)) {
    add({
      label: name,
      kind: "type",
      detail: hlslDetail(name, entry, "datatype"),
      documentation: entry.description,
    });
  }
  for (const [name, entry] of Object.entries(intrinsicfunctions)) {
    add({
      label: name,
      kind: "function",
      detail: hlslDetail(name, entry, "function"),
      documentation: entry.description,
    });
  }
  for (const [name, entry] of Object.entries(semantics)) {
    add({
      label: name,
      kind: "semantic",
      detail: "semantic",
      documentation: entry.description,
    });
  }
  for (const [name, entry] of Object.entries(semanticsNum)) {
    add({
      label: name,
      kind: "semantic",
      detail: "semantic",
      documentation: entry.description,
    });
  }
  for (const [name, entry] of Object.entries(keywords)) {
    add({
      label: name,
      kind: "keyword",
      detail: "keyword",
      documentation: entry.description,
    });
  }
  for (const [name, entry] of Object.entries(preprocessors)) {
    add({
      label: name,
      kind: "keyword",
      detail: "preprocessor",
      documentation: entry.description,
    });
  }

  for (const v of unityBuiltinVariables) {
    add({
      label: v.name,
      kind: "variable",
      detail: `(${v.category}) ${v.type}`,
      documentation: v.description,
    });
  }
  for (const f of unityBuiltinFunctions) {
    add({
      label: f.name,
      kind: "function",
      detail: f.signature,
      documentation: f.description,
    });
  }
  for (const m of unityBuiltinMacros) {
    add({
      label: m.name,
      kind: "constant",
      detail: `(${m.category}) Macro`,
      documentation: m.usage ? `${m.description}\n\n${m.usage}` : m.description,
    });
  }
  for (const k of shaderLabKeywords) {
    add({
      label: k.name,
      kind: "keyword",
      detail: `(ShaderLab) ${k.category}`,
      documentation: k.description,
      insertText: k.snippet ?? k.name,
      snippet: Boolean(k.snippet),
    });
  }
  for (const p of shaderLabPropertyTypes) {
    add({
      label: p.name,
      kind: "type",
      detail: p.description,
      documentation: `${p.description}\n\n${p.example}`,
    });
  }
  for (const p of pragmaDirectives) {
    const label = p.name.replace(/^#pragma\s+/, "");
    add({
      label,
      kind: "keyword",
      detail: p.example,
      documentation: p.description,
      insertText: label,
    });
  }

  for (const v of urpBuiltinVariables) {
    add({
      label: v.name,
      kind: "variable",
      detail: `(URP ${v.category}) ${v.type}`,
      documentation: v.description,
    });
  }
  for (const f of urpBuiltinFunctions) {
    add({
      label: f.name,
      kind: "function",
      detail: `(URP) ${f.signature}`,
      documentation: f.description,
    });
  }
  for (const m of urpBuiltinMacros) {
    add({
      label: m.name,
      kind: "constant",
      detail: `(URP ${m.category}) Macro`,
      documentation: m.usage ? `${m.description}\n\n${m.usage}` : m.description,
    });
  }
  for (const path of urpIncludePaths) {
    add({
      label: path,
      kind: "file",
      detail: "URP include",
      documentation: `#include "${path}"`,
      insertText: `#include "${path}"`,
    });
  }

  cached = out;
  return out;
}
