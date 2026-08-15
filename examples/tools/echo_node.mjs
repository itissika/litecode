#!/usr/bin/env node
/** Custom tool: echo a message from stdin JSON. */
import { stdin } from "node:process";

const chunks = [];
for await (const chunk of stdin) {
  chunks.push(chunk);
}
const raw = Buffer.concat(chunks).toString("utf8");

let data;
try {
  data = JSON.parse(raw || "{}");
} catch (err) {
  console.error(`invalid json input: ${err}`);
  process.exit(1);
}

const message = data?.message;
if (typeof message !== "string" || !message) {
  console.error("missing required string field: message");
  process.exit(1);
}

process.stdout.write(`${message}\n`);
