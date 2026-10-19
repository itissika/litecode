import fs from 'node:fs';
import net from 'node:net';
import { inspect } from 'node:util';
import vm from 'node:vm';
import { makeLibs } from './libs.mjs';

// Capture runtime configuration before the program-visible environment is cleared.
const controlAddress = process.env.LITECODE_CODE_CONTROL_ADDR || '';
const controlToken = process.env.LITECODE_CODE_CONTROL_TOKEN || '';
const toolNames = (process.env.LITECODE_CODE_TOOLS || '')
  .split(',')
  .map((name) => name.trim())
  .filter(Boolean);
const userScriptPath = process.env.LITECODE_CODE_SCRIPT || '';
const maxFrameBytes = Number(process.env.LITECODE_CODE_MAX_FRAME_BYTES || '8388608');
const maxOutputBytes = Number(process.env.LITECODE_CODE_MAX_OUTPUT_BYTES || '1048576');
const MAX_PENDING_TOOL_CALLS = 64;

// Keep only executable startup state in the native environment until the child
// is ready. The program-visible environment is replaced with an empty object.
for (const key of Object.keys(process.env)) delete process.env[key];
process.env = Object.create(null);

if (!controlAddress || !controlToken || !userScriptPath) {
  process.stderr.write('run_code runtime: missing control configuration\n');
  process.exit(2);
}
if (!Number.isSafeInteger(maxFrameBytes) || maxFrameBytes <= 0
  || !Number.isSafeInteger(maxOutputBytes) || maxOutputBytes <= 0) {
  process.stderr.write('run_code runtime: invalid byte limits\n');
  process.exit(2);
}

const separator = controlAddress.lastIndexOf(':');
const controlHost = controlAddress.slice(0, separator);
const controlPort = Number(controlAddress.slice(separator + 1));
if (!controlHost || !Number.isInteger(controlPort) || controlPort <= 0) {
  process.stderr.write('run_code runtime: invalid control address\n');
  process.exit(2);
}

const stderrWrite = process.stderr.write.bind(process.stderr);
const pending = new Map();
let nextId = 0;
let socket;
let inbound = Buffer.alloc(0);
let ready = false;
let finished = false;
let started = false;

function formatError(error) {
  return String(error && error.stack ? error.stack : error);
}

function writeFrame(message) {
  if (!socket || !socket.writable) throw new Error('run_code control channel is not writable');
  const body = Buffer.from(JSON.stringify(message), 'utf8');
  if (body.length === 0 || body.length > maxFrameBytes) {
    throw new Error(`run_code control frame exceeds ${maxFrameBytes} bytes`);
  }
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32BE(body.length, 0);
  socket.write(Buffer.concat([header, body]));
}

function finish(message) {
  if (finished) return;
  finished = true;
  try {
    writeFrame(message);
    socket.end();
    // Give the socket write callback a chance to flush before terminating.
    setTimeout(() => process.exit(0), 20);
  } catch (error) {
    try { stderrWrite(`run_code runtime final frame failed: ${formatError(error)}\n`); } catch {}
    process.exit(1);
  }
}

function finishError(kind, message) {
  const text = String(message);
  finish({
    type: 'final',
    ok: false,
    error_kind: kind,
    error: text.length > 16_000 ? text.slice(0, 16_000) + '\n[error truncated]' : text,
  });
}

function finishValue(value) {
  let serialized;
  try {
    serialized = JSON.stringify(value);
  } catch (error) {
    finishError('invalid_output', `program completion must be JSON-serializable: ${formatError(error)}`);
    return;
  }
  if (serialized === undefined) {
    finish({ type: 'final', ok: true });
    return;
  }
  if (Buffer.byteLength(serialized, 'utf8') > maxOutputBytes) {
    finishError('output_limit', `program completion exceeds ${maxOutputBytes} bytes`);
    return;
  }
  finish({ type: 'final', ok: true, has_value: true, value: JSON.parse(serialized) });
}

function fatal(message) {
  try { stderrWrite(`run_code runtime: ${message}\n`); } catch {}
  process.exit(1);
}

function handleFrame(message) {
  if (message === null || typeof message !== 'object' || Array.isArray(message)) {
    fatal('invalid control frame');
    return;
  }
  if (message.type === 'tool_result') {
    const call = pending.get(String(message.id));
    if (!call) return;
    pending.delete(String(message.id));
    if (message.ok) call.resolve(message.result ?? null);
    else call.reject(new Error(String(message.error || 'tool host error')));
    return;
  }
  if (message.type === 'ready') {
    if (ready || started) return;
    ready = true;
    started = true;
    void startProgram();
    return;
  }
  fatal(`unexpected control message: ${String(message.type)}`);
}

function receiveFrames(chunk) {
  inbound = Buffer.concat([inbound, chunk]);
  for (;;) {
    if (inbound.length < 4) return;
    const bodyBytes = inbound.readUInt32BE(0);
    if (bodyBytes === 0 || bodyBytes > maxFrameBytes) {
      fatal(`invalid control frame length: ${bodyBytes}`);
      return;
    }
    if (inbound.length < 4 + bodyBytes) return;
    const body = inbound.subarray(4, 4 + bodyBytes);
    inbound = inbound.subarray(4 + bodyBytes);
    let message;
    try {
      message = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body));
    } catch (error) {
      fatal(`invalid control JSON: ${formatError(error)}`);
      return;
    }
    handleFrame(message);
    if (finished) return;
  }
}

function callTool(name, args, options) {
  return new Promise((resolve, reject) => {
    if (pending.size >= MAX_PENDING_TOOL_CALLS) {
      reject(new Error(`too many pending tool calls (max ${MAX_PENDING_TOOL_CALLS})`));
      return;
    }
    const id = String(++nextId);
    pending.set(id, { resolve, reject });
    try {
      writeFrame({
        type: 'tool_call',
        id,
        name,
        args: args ?? {},
        options: options ?? null,
      });
    } catch (error) {
      pending.delete(id);
      reject(error);
    }
  });
}

const tools = Object.create(null);
for (const name of toolNames) {
  Object.defineProperty(tools, name, {
    value: (args, options) => callTool(name, args, options),
    enumerable: true,
    writable: false,
    configurable: false,
  });
}
Object.freeze(tools);

const libs = makeLibs();

function consoleWrite(...args) {
  const text = args.map((value) => {
    try {
      return typeof value === 'string' ? value : inspect(value, {
        depth: 4,
        maxArrayLength: 100,
        maxStringLength: 10_000,
      });
    } catch {
      return String(value);
    }
  }).join(' ');
  stderrWrite(text + '\n');
}

async function startProgram() {
  try {
    const source = fs.readFileSync(userScriptPath, 'utf8');
    const context = vm.createContext({
      tools,
      libs,
      console: Object.freeze({
        log: consoleWrite,
        info: consoleWrite,
        warn: consoleWrite,
        error: consoleWrite,
        debug: consoleWrite,
      }),
    });
    const wrapped = `(async () => {\n${source}\n})()`;
    const script = new vm.Script(wrapped, { filename: 'litecode-code.js' });
    const value = await script.runInContext(context);
    finishValue(value);
  } catch (error) {
    finishError('exception', formatError(error));
  }
}

socket = net.createConnection({ host: controlHost, port: controlPort });
socket.setNoDelay(true);
socket.on('connect', () => {
  try {
    writeFrame({ type: 'hello', token: controlToken });
  } catch (error) {
    fatal(`could not send control handshake: ${formatError(error)}`);
  }
});
socket.on('data', receiveFrames);
socket.on('error', (error) => {
  if (!finished) fatal(`control channel error: ${formatError(error)}`);
});
socket.on('close', () => {
  if (!finished) fatal('control channel closed before the program settled');
});
