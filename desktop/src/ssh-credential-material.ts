import fs from "node:fs";
import path from "node:path";

/** Private keys (ed25519/rsa/ecdsa) are a few KB; this caps absurdly large reads. */
const MAX_KEY_BYTES = 256 * 1024;

/**
 * Materialize an SSH_ASKPASS helper that echoes a plaintext password. The
 * caller must remove the returned directory (see removeMaterializedKey); the
 * password lives only inside that throwaway temp directory.
 */
export function materializePasswordAskPass(password: string): { keyDirectory: string; askPassCommand: string } {
  const keyDirectory = fs.mkdtempSync(path.join(getTempDir(), "litecode-ssh-"));
  const passwordFile = path.join(keyDirectory, "password");
  const askPassCommand = path.join(keyDirectory, "askpass.cmd");
  fs.writeFileSync(passwordFile, password, { encoding: "utf8", mode: 0o600 });
  fs.writeFileSync(
    askPassCommand,
    `@echo off\r\ntype "${passwordFile.replace(/"/g, "\"\"")}"\r\n`,
    { encoding: "utf8", mode: 0o700 },
  );
  return { keyDirectory, askPassCommand };
}

export function materializePrivateKeyFile(keyContents: string): {
  keyDirectory: string;
  keyFile: string;
} {
  const keyDirectory = fs.mkdtempSync(path.join(getTempDir(), "litecode-ssh-"));
  const keyFile = path.join(keyDirectory, "identity");
  fs.writeFileSync(keyFile, keyContents, { encoding: "utf8", mode: 0o600 });
  return { keyDirectory, keyFile };
}

/** Recursively remove a materialized temp key directory, if any. */
export function removeMaterializedKey(keyDirectory?: string): void {
  if (!keyDirectory) return;
  fs.rmSync(keyDirectory, { recursive: true, force: true });
}

/**
 * Run `body` with a materialized key directory and always remove that
 * directory when the operation finishes, including on a thrown error.
 * Use for operations where the temp material is scoped to a single call.
 */
export async function withMaterializedKeyDirectory<T>(
  keyDirectory: string | undefined,
  body: () => Promise<T>,
): Promise<T> {
  try {
    return await body();
  } finally {
    removeMaterializedKey(keyDirectory);
  }
}

/**
 * Read a local SSH private key file, rejecting non-regular files and
 * oversized reads (DESK-02 arbitrary-path read control). The key may live
 * anywhere on disk; this bounds what can be read through the IPC handler.
 */
export function readPrivateKeyFile(identityFile: string): string {
  if (!identityFile || typeof identityFile !== "string" || /[\0\r\n]/.test(identityFile)) {
    throw new Error("SSH identity file must be a non-empty local path.");
  }
  let stat;
  try {
    stat = fs.statSync(identityFile);
  } catch {
    throw new Error(`SSH identity file is not readable: ${identityFile}`);
  }
  if (!stat.isFile()) {
    throw new Error(`SSH identity file must be a regular file: ${identityFile}`);
  }
  if (stat.size > MAX_KEY_BYTES) {
    throw new Error(`SSH identity file is too large to be a private key: ${identityFile}`);
  }
  return fs.readFileSync(identityFile, "utf8");
}

function getTempDir(): string {
  return process.env.TMPDIR || process.env.TMP || process.env.TEMP || "/tmp";
}
