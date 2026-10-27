import crypto from "node:crypto";
import http from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";

const MAX_BODY_BYTES = 128 * 1024;

/** A domain failure the bridge can return without leaking an implementation detail. */
export class BrowserBridgeError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status = 400,
  ) {
    super(message);
    this.name = "BrowserBridgeError";
  }
}

/** The browser actions the HTTP bridge exposes. Results are serialized as data. */
export type BrowserBridgeHandlers = {
  open(input: Record<string, unknown>): Promise<unknown>;
  close(input: Record<string, unknown>): Promise<unknown>;
  list(): unknown;
  read(input: Record<string, unknown>): Promise<unknown>;
  click(input: Record<string, unknown>): Promise<unknown>;
  type(input: Record<string, unknown>): Promise<unknown>;
};

export type BrowserBridge = {
  /** Loopback base URL, e.g. http://127.0.0.1:43121 */
  url: string;
  token: string;
  close(): Promise<void>;
};

type Route = {
  method: "GET" | "POST";
  handler: (input: Record<string, unknown>) => Promise<unknown> | unknown;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function authorized(header: string | undefined, token: string): boolean {
  if (typeof header !== "string") return false;
  const expected = Buffer.from(`Bearer ${token}`);
  const actual = Buffer.from(header);
  if (expected.length !== actual.length) return false;
  return crypto.timingSafeEqual(expected, actual);
}

async function readBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of req) {
    const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += buf.length;
    if (size > MAX_BODY_BYTES) {
      throw new BrowserBridgeError("invalid_input", "Request body is too large", 413);
    }
    chunks.push(buf);
  }
  if (chunks.length === 0) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new BrowserBridgeError("invalid_input", "Request body must be JSON", 400);
  }
  if (!isRecord(parsed)) {
    throw new BrowserBridgeError("invalid_input", "Request body must be an object", 400);
  }
  return parsed;
}

function respond(res: ServerResponse, status: number, body: unknown): void {
  if (res.writableEnded) return;
  const text = JSON.stringify(body);
  res.statusCode = status;
  res.setHeader("content-type", "application/json; charset=utf-8");
  res.setHeader("cache-control", "no-store");
  res.setHeader("content-length", Buffer.byteLength(text).toString());
  res.end(text);
}

function routeFor(
  pathname: string,
  method: string | undefined,
  handlers: BrowserBridgeHandlers,
): Route | null {
  if (pathname === "/list" && method === "GET") {
    return { method: "GET", handler: () => handlers.list() };
  }
  if (method !== "POST") return null;
  switch (pathname) {
    case "/open":
      return { method: "POST", handler: (input) => handlers.open(input) };
    case "/close":
      return { method: "POST", handler: (input) => handlers.close(input) };
    case "/read":
      return { method: "POST", handler: (input) => handlers.read(input) };
    case "/click":
      return { method: "POST", handler: (input) => handlers.click(input) };
    case "/type":
      return { method: "POST", handler: (input) => handlers.type(input) };
    default:
      return null;
  }
}

export async function startBrowserBridge(
  handlers: BrowserBridgeHandlers,
  token = crypto.randomBytes(32).toString("base64url"),
): Promise<BrowserBridge> {
  const server = http.createServer((req, res) => {
    void (async () => {
      try {
        const url = new URL(req.url ?? "/", "http://127.0.0.1");
        if (!authorized(req.headers.authorization, token)) {
          respond(res, 401, {
            ok: false,
            code: "unauthorized",
            message: "Browser bridge authorization failed",
          });
          return;
        }
        const route = routeFor(url.pathname, req.method, handlers);
        if (!route) {
          respond(res, 404, {
            ok: false,
            code: "not_found",
            message: "Unknown browser bridge route",
          });
          return;
        }
        const input = route.method === "POST" ? await readBody(req) : {};
        const data = await route.handler(input);
        respond(res, 200, { ok: true, data });
      } catch (error) {
        if (error instanceof BrowserBridgeError) {
          respond(res, error.status, {
            ok: false,
            code: error.code,
            message: error.message,
          });
          return;
        }
        console.error("browser bridge request failed", error);
        respond(res, 500, {
          ok: false,
          code: "internal",
          message: "Browser bridge request failed",
        });
      }
    })();
  });

  server.on("clientError", (_error, socket) => {
    if (socket.writable) {
      socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
    }
  });

  await new Promise<void>((resolve, reject) => {
    const onError = (error: Error) => {
      server.off("listening", onListening);
      reject(error);
    };
    const onListening = () => {
      server.off("error", onError);
      resolve();
    };
    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(0, "127.0.0.1");
  });

  const address = server.address();
  if (!address || typeof address === "string") {
    server.close();
    throw new Error("Browser bridge did not bind a loopback port");
  }

  return {
    url: `http://127.0.0.1:${address.port}`,
    token,
    close: async () => {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
