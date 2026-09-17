import type * as Monaco from "monaco-editor";
import type { HighlighterCore } from "shiki/core";
import { INITIAL, type StateStack } from "shiki/textmate";

import { getMarkdownHighlighter } from "./shiki";
import { shaderSuggests, type ShaderSuggestKind } from "./shader/completions";

export const SHADER_LANGUAGE_IDS = ["hlsl", "shaderlab"] as const;

const C_LIKE: Monaco.languages.LanguageConfiguration = {
  comments: { lineComment: "//", blockComment: ["/*", "*/"] },
  brackets: [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
  ],
  autoClosingPairs: [
    { open: "{", close: "}" },
    { open: "[", close: "]" },
    { open: "(", close: ")" },
    { open: '"', close: '"' },
  ],
  surroundingPairs: [
    { open: "{", close: "}" },
    { open: "[", close: "]" },
    { open: "(", close: ")" },
    { open: '"', close: '"' },
  ],
};

class TokenizerState implements Monaco.languages.IState {
  constructor(readonly ruleStack: StateStack) {}
  clone(): TokenizerState {
    return new TokenizerState(this.ruleStack);
  }
  equals(other: Monaco.languages.IState): boolean {
    return other instanceof TokenizerState && this.ruleStack.equals(other.ruleStack);
  }
}

function kindOf(
  monaco: typeof Monaco,
  kind: ShaderSuggestKind,
): Monaco.languages.CompletionItemKind {
  const K = monaco.languages.CompletionItemKind;
  switch (kind) {
    case "function":
      return K.Function;
    case "type":
      return K.Struct;
    case "variable":
      return K.Variable;
    case "constant":
      return K.Constant;
    case "file":
      return K.File;
    case "semantic":
      return K.Reference;
    default:
      return K.Keyword;
  }
}

function shikiTokensProvider(
  highlighter: HighlighterCore,
  lang: string,
): Monaco.languages.TokensProvider {
  const grammar = highlighter.getLanguage(lang);
  return {
    getInitialState: () => new TokenizerState(INITIAL),
    tokenize(line, state) {
      const stack = state instanceof TokenizerState ? state.ruleStack : INITIAL;
      const result = grammar.tokenizeLine(line, stack);
      return {
        endState: new TokenizerState(result.ruleStack),
        tokens: result.tokens.map((token) => ({
          startIndex: token.startIndex,
          scopes: token.scopes[token.scopes.length - 1] ?? "",
        })),
      };
    },
  };
}

export function registerShaderSupport(monaco: typeof Monaco): void {
  monaco.languages.register({
    id: "hlsl",
    extensions: [".hlsl", ".hlsli", ".fx", ".fxh", ".compute", ".cginc", ".usf", ".ush", ".cg"],
    aliases: ["HLSL"],
  });
  monaco.languages.register({
    id: "shaderlab",
    extensions: [".shader"],
    aliases: ["ShaderLab", "shader"],
  });

  const completion: Monaco.languages.CompletionItemProvider = {
    triggerCharacters: [".", "#"],
    provideCompletionItems(model, position) {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };
      return {
        suggestions: shaderSuggests().map((item) => {
          const suggestion: Monaco.languages.CompletionItem = {
            label: item.label,
            kind: kindOf(monaco, item.kind),
            insertText: item.insertText ?? item.label,
            range,
            detail: item.detail,
            documentation: item.documentation,
          };
          if (item.snippet) {
            suggestion.insertTextRules =
              monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet;
          }
          return suggestion;
        }),
      };
    },
  };

  for (const id of SHADER_LANGUAGE_IDS) {
    monaco.languages.setLanguageConfiguration(id, C_LIKE);
    monaco.languages.registerCompletionItemProvider(id, completion);
  }

  void getMarkdownHighlighter()
    .then((highlighter) => {
      for (const id of SHADER_LANGUAGE_IDS) {
        monaco.languages.setTokensProvider(id, shikiTokensProvider(highlighter, id));
      }
    })
    .catch((err) => {
      console.warn("shader shiki tokenizer failed", err);
    });
}
