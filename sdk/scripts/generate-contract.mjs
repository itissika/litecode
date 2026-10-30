import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import { Project, SyntaxKind } from "ts-morph";
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

const suiteOrder = ["file", "search", "process", "web", "memory", "browser", "lsp"];
const project = new Project({ tsConfigFilePath: tsconfigPath });
const apiSource = project.getSourceFileOrThrow(apiPath);
const toolsInterface = apiSource.getInterfaceOrThrow("Tools");

const typeNodes = new Map();
for (const node of [...apiSource.getInterfaces(), ...apiSource.getTypeAliases()]) {
  const name = node.getName();
  if (name === "Tools" || name === "MemoryTools") continue;
  const refs = new Set();
  for (const ref of node.getDescendantsOfKind(SyntaxKind.TypeReference)) {
    const text = ref.getTypeName().getText();
    const last = text.split(".").pop();
    if (last) refs.add(last);
  }
  typeNodes.set(name, { text: node.getFullText().trim(), refs: [...refs] });
}

const typeDecls = {};
for (const name of [...typeNodes.keys()].sort()) {
  typeDecls[name] = typeNodes.get(name).text;
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

function collectMethod(iface, pathParts, out) {
  for (const method of iface.getMethods()) {
    const tags = tagMap(method);
    const path = [...pathParts, method.getName()];
    const tool = tags.get("tool") || path.join(".");
    const suite = tags.get("suite") || pathParts[0] || "file";
    const inputType = tags.get("input");
    const outputType = tags.get("output");
    if (!inputType || !outputType) {
      throw new Error(`method ${tool} is missing @input/@output tags`);
    }
    if (!typeNodes.has(inputType)) throw new Error(`unknown input type ${inputType} for ${tool}`);
    if (!typeNodes.has(outputType)) throw new Error(`unknown output type ${outputType} for ${tool}`);
    out.push({
      tool,
      suite,
      path,
      doc: methodDoc(method),
      signature: method.getText().trim(),
      input_type: inputType,
      output_type: outputType,
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

function collectDeps(root, out) {
  if (out.has(root)) return;
  const node = typeNodes.get(root);
  if (!node) return;
  out.add(root);
  for (const ref of node.refs) collectDeps(ref, out);
}

const generator = createGenerator({
  path: apiPath,
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
    if (!value.$ref.startsWith(prefix)) throw new Error(`unsupported $ref ${value.$ref}`);
    const name = decodeURIComponent(value.$ref.slice(prefix.length));
    if (stack.includes(name)) throw new Error(`recursive schema ref ${stack.concat(name).join(" -> ")}`);
    const def = defs[name];
    if (!def) throw new Error(`missing definition ${name}`);
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

function schemaFor(typeName) {
  const schema = generator.createSchema(typeName);
  return inlineRefs(schema, schema.definitions ?? {});
}

const tools = {};
for (const method of methods) {
  const deps = new Set();
  collectDeps(method.input_type, deps);
  collectDeps(method.output_type, deps);
  tools[method.tool] = {
    path: method.path,
    suite: method.suite,
    tool: method.tool,
    doc: method.doc,
    signature: method.signature,
    input_type: method.input_type,
    output_type: method.output_type,
    type_deps: [...deps].sort(),
    input_schema: schemaFor(method.input_type),
    output_schema: schemaFor(method.output_type),
  };
}

function renderDoc(doc, indent) {
  if (!doc) return "";
  const lines = doc.trim().split(/\r?\n/);
  const prefix = `${indent} *`;
  return `${indent}/**\n${lines.map((line) => (line.trim() ? `${prefix} ${line.trim()}` : prefix)).join("\n")}\n${indent} */\n`;
}

function renderToolEntry(tool, indent) {
  return `${renderDoc(tool.doc, indent)}${indent}${tool.signature}`;
}

function renderToolsInterface(entries) {
  const flat = entries.filter((tool) => tool.path.length === 1);
  const memory = entries.filter((tool) => tool.path.length === 2 && tool.path[0] === "memory");
  const lines = ["interface Tools {"];
  const bySuite = new Map();
  for (const tool of flat) {
    if (!bySuite.has(tool.suite)) bySuite.set(tool.suite, []);
    bySuite.get(tool.suite).push(tool);
  }
  for (const suite of suiteOrder) {
    const group = bySuite.get(suite);
    if (!group || group.length === 0) continue;
    lines.push(`  // ${suite}`);
    for (const tool of group) lines.push(renderToolEntry(tool, "  "));
    lines.push("");
  }
  if (memory.length > 0) {
    lines.push("  // memory");
    lines.push("  memory: {");
    for (const tool of memory) lines.push(renderToolEntry(tool, "    "));
    lines.push("  };");
    lines.push("");
  }
  if (lines[lines.length - 1] === "") lines.pop();
  lines.push("}");
  return lines.join("\n");
}

function renderTsSdk(entries) {
  const deps = new Set();
  for (const tool of entries) for (const dep of tool.type_deps) deps.add(dep);
  const types = [...deps].sort().map((name) => typeDecls[name]).filter(Boolean).join("\n\n");
  const preludeDts = fs.readFileSync(preludeDtsPath, "utf8").trim();
  const libsDts = fs.readFileSync(libsDtsPath, "utf8").trim();
  return `${preludeDts}\n\n${libsDts}\n\n${types}\n\n${renderToolsInterface(entries)}\n\ndeclare const tools: Tools;\n`;
}

function renderPromptSdk(entries) {
  const preludeMd = fs.readFileSync(preludeMdPath, "utf8").trim();
  return `${preludeMd}\n\n\`\`\`ts\n${renderTsSdk(entries)}\`\`\`\n`;
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
  prose: fs.readFileSync(preludeMdPath, "utf8").trim(),
  declarations: fs.readFileSync(preludeDtsPath, "utf8").trim(),
  libs: fs.readFileSync(libsDtsPath, "utf8").trim(),
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
      console.error(`drift: ${rel}`);
      failed = true;
    }
  } else {
    fs.writeFileSync(file, content, "utf8");
    console.log(`wrote ${rel}`);
  }
}
if (failed) process.exit(1);
console.log(`tools=${entries.length} sdk_hash=${sdkHash}`);