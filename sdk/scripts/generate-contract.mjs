import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import { Node, Project, SyntaxKind } from "ts-morph";
import { createGenerator } from "ts-json-schema-generator";

const here = path.dirname(fileURLToPath(import.meta.url));
const sdkDir = path.resolve(here, "..");
const repoRoot = path.resolve(sdkDir, "..");
const apiPath = path.join(sdkDir, "src", "api.d.ts");
const preludeMdPath = path.join(sdkDir, "src", "prelude.md");
const preludeDtsPath = path.join(sdkDir, "src", "prelude.d.ts");
const libsDtsPath = path.join(sdkDir, "src", "libs.d.ts");
const tsconfigPath = path.join(sdkDir, "tsconfig.json");
const generatedDir = path.join(sdkDir, "generated");
const contractPath = path.join(generatedDir, "tool-contract.json");
const fragmentsPath = path.join(generatedDir, "sdk-fragments.json");
const allSdkPath = path.join(generatedDir, "all-code-sdk.d.ts");
const allPromptPath = path.join(generatedDir, "all-code-sdk.txt");
const check = process.argv.includes("--check");
const EOL_RE = /\r\n?/g;

/** Read one SDK source as LF: generated output must not depend on the checkout EOL. */
function readSdkSource(file) {
  return fs.readFileSync(file, "utf8").replace(EOL_RE, "\n");
}

const suiteOrder = ["file", "search", "process", "web", "browser", "lsp", "memory"];
const project = new Project({ tsConfigFilePath: tsconfigPath });
const apiSource = project.getSourceFileOrThrow(apiPath);
const apiText = readSdkSource(apiPath);
const toolsInterface = apiSource.getInterfaceOrThrow("Tools");

/** Every named type declared in api.d.ts, with the type names it references. */
const typeNodes = new Map();
for (const node of [...apiSource.getInterfaces(), ...apiSource.getTypeAliases()]) {
  const name = node.getName();
  if (name === "Tools") continue;
  const refs = new Set();
  for (const ref of node.getDescendantsOfKind(SyntaxKind.TypeReference)) {
    const text = ref.getTypeName().getText();
    const last = text.split(".").pop();
    if (last) refs.add(last);
  }
  typeNodes.set(name, { node, refs: [...refs] });
}

function tagMap(method) {
  const docs = method.getJsDocs();
  const tags = new Map();
  for (const doc of docs) {
    for (const tag of doc.getTags()) {
      tags.set(tag.getTagName(), (tag.getCommentText() ?? "").trim());
    }
  }
  return tags;
}

function methodDoc(method) {
  return method
    .getJsDocs()
    .map((doc) => doc.getDescription().trim())
    .filter(Boolean)
    .join("\n\n");
}

function indentOf(line) {
  let index = 0;
  while (index < line.length && (line[index] === " " || line[index] === "\t")) index += 1;
  return line.slice(0, index);
}

/** Method text with its own declaration indent removed, so renderers can re-indent it. */
function relativeSignature(method) {
  const full = method.getSourceFile().getFullText();
  const lineStart = full.lastIndexOf("\n", method.getStart() - 1) + 1;
  const base = indentOf(full.slice(lineStart, method.getStart()));
  return method
    .getText()
    .split("\n")
    .map((line) => (line.trimEnd() && line.startsWith(base) ? line.slice(base.length).trimEnd() : line.trimEnd()))
    .join("\n");
}

function collectMethod(iface, pathParts, out) {
  for (const method of iface.getMethods()) {
    const tags = tagMap(method);
    const memberPath = [...pathParts, method.getName()];
    const tool = tags.get("tool") || memberPath.join(".");
    const suite = tags.get("suite") || pathParts[0] || "file";
    const param = method.getParameters()[0];
    if (!param) throw new Error("method " + tool + " has no args parameter");
    const returnNode = method.getReturnTypeNodeOrThrow();
    const promiseArgs = returnNode.getTypeArguments();
    const resultArgs = promiseArgs.length === 1 ? promiseArgs[0].getTypeArguments() : [];
    if (resultArgs.length !== 1) {
      throw new Error("method " + tool + " must return Promise<ToolResult<T>>");
    }
    out.push({
      tool,
      suite,
      path: memberPath,
      doc: methodDoc(method),
      signature: relativeSignature(method),
      input_node: param.getTypeNodeOrThrow(),
      output_node: resultArgs[0],
    });
  }
  for (const prop of iface.getProperties()) {
    const typeName = prop.getTypeNodeOrThrow().getText();
    const nested = apiSource.getInterface(typeName);
    if (!nested) continue;
    collectMethod(nested, [...pathParts, prop.getName()], out);
  }
}

const methods = [];
collectMethod(toolsInterface, [], methods);
methods.sort((a, b) => {
  const ai = suiteOrder.indexOf(a.suite);
  const bi = suiteOrder.indexOf(b.suite);
  return (ai < 0 ? 999 : ai) - (bi < 0 ? 999 : bi) || a.tool.localeCompare(b.tool);
});

function typeRefNames(node) {
  const refs = [];
  if (node.getKind() === SyntaxKind.TypeReference) refs.push(node);
  refs.push(...node.getDescendantsOfKind(SyntaxKind.TypeReference));
  return refs
    .map((ref) => ref.getTypeName().getText().split(".").pop())
    .filter(Boolean);
}

/** Named types reachable from one type node, following references through declarations. */
function closureOf(node) {
  const names = new Set();
  const visit = (name) => {
    if (names.has(name)) return;
    const entry = typeNodes.get(name);
    if (!entry) return;
    names.add(name);
    for (const ref of entry.refs) visit(ref);
  };
  for (const name of typeRefNames(node)) visit(name);
  return names;
}

// A type used by two or more members stays a named public type; every other
// type is written into the member that uses it.
const publicNames = new Set();
{
  const counts = new Map();
  for (const method of methods) {
    const names = new Set([...closureOf(method.input_node), ...closureOf(method.output_node)]);
    for (const name of names) counts.set(name, (counts.get(name) || 0) + 1);
  }
  for (const [name, count] of counts) if (count >= 2) publicNames.add(name);
}

function aliasName(tool, side) {
  let out = "";
  for (const ch of tool + "_" + side) out += /[A-Za-z0-9]/.test(ch) ? ch : "_";
  return "__" + out;
}

/** Strip a `//`, `/*` or `/**` comment down to its text. */
function cleanComment(raw) {
  if (raw.startsWith("//")) {
    let text = raw.slice(2);
    if (text.startsWith(" ")) text = text.slice(1);
    return text.trim();
  }
  let body = raw;
  if (body.startsWith("/**")) body = body.slice(3);
  else if (body.startsWith("/*")) body = body.slice(2);
  if (body.endsWith("*/")) body = body.slice(0, -2);
  const lines = body.split("\n").map((line) => {
    let text = line.trim();
    if (text.startsWith("*")) text = text.slice(1).trim();
    return text;
  });
  while (lines.length && lines[0] === "") lines.shift();
  while (lines.length && lines[lines.length - 1] === "") lines.pop();
  return lines.join("\n");
}

/** The doc a node carries: line comment after it, line comment above it, or JSDoc. */
function docOf(node, anchor) {
  const full = node.getSourceFile().getFullText();
  const end = node.getEnd();
  const lineEnd = full.indexOf("\n", end);
  const tail = full.slice(end, lineEnd < 0 ? full.length : lineEnd).trim();
  if (tail.startsWith("//")) return cleanComment(tail);
  const start = anchor === undefined ? node.getStart() : anchor;
  const lineStart = full.lastIndexOf("\n", start - 1) + 1;
  const ownLine = full.slice(lineStart, start).trim() === "";
  const prevStart = full.lastIndexOf("\n", lineStart - 2) + 1;
  const prevLine = full.slice(prevStart, lineStart).trim();
  if (ownLine && prevLine.startsWith("//")) return cleanComment(prevLine);
  if (typeof node.getJsDocs === "function") {
    const docs = node.getJsDocs();
    if (docs.length) {
      const doc = docs[docs.length - 1];
      return cleanComment(full.slice(doc.getStart(), doc.getEnd()));
    }
  }
  return null;
}

function jsdocLines(text, indent) {
  if (!text.includes("\n")) return [indent + "/** " + text + " */"];
  const out = [indent + "/**"];
  for (const line of text.split("\n")) out.push(indent + " *" + (line ? " " + line : ""));
  out.push(indent + " */");
  return out;
}

let hoisted = 0;
const scratchAliases = [];

/** Print one type expression as schema source: JSDoc instead of line comments. */
function printType(node, base) {
  const kind = node.getKind();
  if (kind === SyntaxKind.TypeLiteral) {
    const out = ["{"];
    const fieldIndent = base + "  ";
    for (const member of node.getMembers()) {
      if (!Node.isPropertySignature(member)) {
        throw new Error("unsupported member " + member.getKindName());
      }
      const doc = docOf(member, member.getNameNode().getStart());
      if (doc !== null) out.push(...jsdocLines(doc, fieldIndent));
      const name = member.getName() + (member.hasQuestionToken() ? "?" : "");
      const type = printType(member.getTypeNodeOrThrow(), fieldIndent);
      out.push(fieldIndent + name + ": " + type + ";");
    }
    out.push(base + "}");
    const body = out.join("\n");
    const doc = docOf(node);
    if (doc === null) return body;
    const alias = "__h" + hoisted;
    hoisted += 1;
    scratchAliases.push("/** " + doc + " */\ntype " + alias + " = " + body + ";");
    return alias;
  }
  if (kind === SyntaxKind.ParenthesizedType) {
    // Parentheses mark a union that keeps its own alias, so the schema stays a named union.
    const alias = "__h" + hoisted;
    hoisted += 1;
    scratchAliases.push("type " + alias + " = " + printType(node.getTypeNode(), base) + ";");
    return alias;
  }
  if (kind === SyntaxKind.TypeReference) {
    const args = node.getTypeArguments();
    if (args.length === 0) return node.getText();
    const name = node.getTypeName().getText();
    if (name === "Array" && args.length === 1) return printType(args[0], base) + "[]";
    return name + "<" + args.map((arg) => printType(arg, base)).join(", ") + ">";
  }
  if (kind === SyntaxKind.ArrayType) {
    return printType(node.getElementTypeNode(), base) + "[]";
  }
  if (kind === SyntaxKind.UnionType) {
    return node.getTypeNodes().map((child) => printType(child, base)).join(" | ");
  }
  return node.getText();
}

function typeSpec(node, tool, side) {
  if (node.getKind() === SyntaxKind.TypeReference) {
    const name = node.getTypeName().getText();
    if (!publicNames.has(name)) {
      throw new Error("method " + tool + " references non-public type " + name);
    }
    // A doc on the reference belongs to the result value, so keep it on a named alias.
    const doc = docOf(node);
    if (doc !== null) {
      const alias = aliasName(tool, side);
      scratchAliases.push("/** " + doc + " */\ntype " + alias + " = " + name + ";");
      return { name: "(inline)", alias };
    }
    return { name, alias: null };
  }
  if (node.getKind() !== SyntaxKind.TypeLiteral) {
    throw new Error("method " + tool + " must use an inline object literal or a public type");
  }
  const alias = aliasName(tool, side);
  scratchAliases.push("type " + alias + " = " + printType(node, "") + ";");
  return { name: "(inline)", alias };
}

const specs = new Map();
for (const method of methods) {
  const input = typeSpec(method.input_node, method.tool, "input");
  const output = typeSpec(method.output_node, method.tool, "output");
  specs.set(method.tool, { input, output });
}

const scratchDir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-sdk-"));
const scratchPath = path.join(scratchDir, "api.d.ts");
fs.writeFileSync(scratchPath, apiText + "\n" + scratchAliases.join("\n\n") + "\n", "utf8");

const generator = createGenerator({
  path: scratchPath,
  tsconfig: tsconfigPath,
  expose: "all",
  topRef: false,
  jsDoc: "extended",
  additionalProperties: false,
  sortProps: true,
  skipTypeCheck: false,
});

function inlineRefs(value, defs, stack = []) {
  if (Array.isArray(value)) return value.map((item) => inlineRefs(item, defs, stack));
  if (!value || typeof value !== "object") return value;
  if (typeof value.$ref === "string") {
    const prefix = "#/definitions/";
    if (!value.$ref.startsWith(prefix)) throw new Error("unsupported $ref " + value.$ref);
    const name = decodeURIComponent(value.$ref.slice(prefix.length));
    if (stack.includes(name)) throw new Error("recursive schema ref " + stack.concat(name).join(" -> "));
    const def = defs[name];
    if (!def) throw new Error("missing definition " + name);
    const siblings = { ...value };
    delete siblings.$ref;
    delete siblings.$schema;
    return { ...inlineRefs(def, defs, stack.concat(name)), ...inlineRefs(siblings, defs, stack) };
  }
  const out = {};
  for (const [key, child] of Object.entries(value)) {
    if (key === "definitions" || key === "$schema") continue;
    out[key] = inlineRefs(child, defs, stack);
  }
  return out;
}

function rawSchema(spec) {
  const schema = generator.createSchema(spec.alias || spec.name);
  return inlineRefs(schema, schema.definitions ?? {});
}

function outputSchemaFor(spec) {
  const schema = rawSchema(spec);
  if (!schema || typeof schema !== "object" || (!schema.properties && !schema.type)) {
    throw new Error("output type " + (spec.alias || spec.name) + " must project to a JSON object");
  }
  const properties = {
    ...(schema.properties ?? {}),
    ok: { type: "boolean", const: true },
  };
  const required = ["ok", ...(schema.required ?? []).filter((name) => name !== "ok")];
  return { ...schema, properties, required };
}

const tools = {};
for (const method of methods) {
  const { input, output } = specs.get(method.tool);
  const deps = new Set();
  for (const node of [method.input_node, method.output_node]) {
    for (const name of closureOf(node)) if (publicNames.has(name)) deps.add(name);
  }
  tools[method.tool] = {
    path: method.path,
    suite: method.suite,
    tool: method.tool,
    doc: method.doc,
    signature: method.signature,
    input_type: input.name,
    output_type: output.name,
    type_deps: [...deps].sort(),
    input_schema: rawSchema(input),
    output_schema: outputSchemaFor(output),
  };
}

if (process.env.KEEP_SCRATCH) console.log("scratch: " + scratchPath);
else
try {
  fs.rmSync(scratchDir, { recursive: true, force: true });
} catch (error) {
  void error;
}

const typeDecls = {};
for (const name of [...publicNames].sort()) {
  const entry = typeNodes.get(name);
  typeDecls[name] = entry.node.getFullText().trim();
}

function renderDoc(doc, indent) {
  if (!doc) return "";
  const lines = doc.trim().split(EOL_RE);
  if (lines.length === 1) return indent + "/** " + lines[0].trim() + " */\n";
  const prefix = indent + " *";
  const body = lines.map((line) => (line.trim() ? prefix + " " + line.trim() : prefix)).join("\n");
  return indent + "/**\n" + body + "\n" + indent + " */\n";
}

function indentBlock(text, indent) {
  return text.split("\n").map((line) => indent + line).join("\n");
}

function renderToolEntry(tool, indent) {
  return renderDoc(tool.doc, indent) + indentBlock(tool.signature, indent);
}

function renderToolsInterface(entries) {
  const lines = ["interface Tools {"];
  const flatBySuite = new Map();
  const nestedBySuite = new Map();
  for (const tool of entries) {
    if (tool.path.length === 1) {
      if (!flatBySuite.has(tool.suite)) flatBySuite.set(tool.suite, []);
      flatBySuite.get(tool.suite).push(tool);
      continue;
    }
    if (tool.path.length !== 2) {
      throw new Error("unsupported SDK path depth: " + tool.path.join("."));
    }
    const suite = tool.path[0];
    if (!nestedBySuite.has(suite)) nestedBySuite.set(suite, []);
    nestedBySuite.get(suite).push(tool);
  }
  const orderedSuites = [
    ...suiteOrder.filter((suite) => nestedBySuite.has(suite)),
    ...[...nestedBySuite.keys()]
      .filter((suite) => !suiteOrder.includes(suite))
      .sort((left, right) => left.localeCompare(right)),
  ];
  for (const suite of suiteOrder) {
    const group = flatBySuite.get(suite);
    if (!group || group.length === 0) continue;
    for (const tool of group) lines.push(renderToolEntry(tool, "  "));
    lines.push("");
  }
  for (const suite of [...flatBySuite.keys()].filter((suite) => !suiteOrder.includes(suite)).sort()) {
    const group = flatBySuite.get(suite);
    for (const tool of group) lines.push(renderToolEntry(tool, "  "));
    lines.push("");
  }
  for (const suite of orderedSuites) {
    lines.push("  " + suite + ": {");
    for (const tool of nestedBySuite.get(suite)) lines.push(renderToolEntry(tool, "    "));
    lines.push("  };");
  }
  if (lines[lines.length - 1] === "") lines.pop();
  lines.push("}");
  return lines.join("\n");
}

function renderTsSdk(entries) {
  const deps = new Set();
  for (const tool of entries) for (const dep of tool.type_deps) deps.add(dep);
  const types = [...deps].sort().map((name) => typeDecls[name]).filter(Boolean).join("\n");
  const preludeDts = readSdkSource(preludeDtsPath).trim();
  const libsDts = readSdkSource(libsDtsPath).trim();
  return [
    preludeDts,
    libsDts,
    types,
    renderToolsInterface(entries),
    "declare const tools: Tools;",
  ].join("\n\n") + "\n";
}

function renderPromptSdk(entries) {
  const preludeMd = readSdkSource(preludeMdPath).trim();
  return preludeMd + "\n\n```ts\n" + renderTsSdk(entries) + "```\n";
}

const entries = Object.values(tools).sort((a, b) => a.path.join(".").localeCompare(b.path.join(".")));
const fullTs = renderTsSdk(entries);
const fullPromptSdk = renderPromptSdk(entries);
const sdkHash = crypto.createHash("sha256").update(fullPromptSdk).digest("hex");

const contract = {
  version: 1,
  sdk_hash: sdkHash,
  tools,
};
const fragments = {
  version: 1,
  sdk_hash: sdkHash,
  prose: readSdkSource(preludeMdPath).trim(),
  declarations: readSdkSource(preludeDtsPath).trim(),
  libs: readSdkSource(libsDtsPath).trim(),
  types: typeDecls,
  tools: Object.fromEntries(
    Object.entries(tools).map(([name, tool]) => [
      name,
      {
        path: tool.path,
        suite: tool.suite,
        doc: tool.doc,
        signature: tool.signature,
        type_deps: tool.type_deps,
      },
    ]),
  ),
};

const outputs = {
  [contractPath]: JSON.stringify(contract, null, 2) + "\n",
  [fragmentsPath]: JSON.stringify(fragments, null, 2) + "\n",
  [allSdkPath]: fullTs,
  [allPromptPath]: fullPromptSdk,
};

fs.mkdirSync(generatedDir, { recursive: true });
let failed = false;
for (const [file, content] of Object.entries(outputs)) {
  const rel = path.relative(repoRoot, file);
  if (check) {
    const existing = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
    if (existing !== content) {
      console.error("drift: " + rel);
      failed = true;
    }
  } else {
    fs.writeFileSync(file, content, "utf8");
    console.log("wrote " + rel);
  }
}
if (failed) process.exit(1);
console.log("tools=" + entries.length + " sdk_hash=" + sdkHash);
