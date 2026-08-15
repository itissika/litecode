import { readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";

import { app, safeStorage } from "electron";

type EncryptedSecrets = Record<string, string>;

function secretFile(): string {
  return path.join(app.getPath("userData"), "ssh-secrets.json");
}

async function readSecrets(): Promise<EncryptedSecrets> {
  try {
    const raw = await readFile(secretFile(), "utf8");
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as EncryptedSecrets) : {};
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return {};
    throw error;
  }
}

async function writeSecrets(secrets: EncryptedSecrets): Promise<void> {
  const file = secretFile();
  const temporary = `${file}.tmp`;
  await writeFile(temporary, JSON.stringify(secrets), { encoding: "utf8", mode: 0o600 });
  await rename(temporary, file);
}

/**
 * Keeps only an encrypted blob in the Litecode profile. On Windows Electron's
 * safeStorage is backed by DPAPI, so the secret is bound to the OS user.
 */
export async function setSecret(id: string, value: string): Promise<void> {
  if (!safeStorage.isEncryptionAvailable()) {
    throw new Error("System credential encryption is unavailable on this device.");
  }
  const secrets = await readSecrets();
  secrets[id] = safeStorage.encryptString(value).toString("base64");
  await writeSecrets(secrets);
}

export async function getSecret(id: string): Promise<string | null> {
  const secrets = await readSecrets();
  const encoded = secrets[id];
  if (!encoded) return null;
  if (!safeStorage.isEncryptionAvailable()) {
    throw new Error("System credential encryption is unavailable on this device.");
  }
  return safeStorage.decryptString(Buffer.from(encoded, "base64"));
}

export async function deleteSecret(id: string): Promise<void> {
  const secrets = await readSecrets();
  if (!(id in secrets)) return;
  delete secrets[id];
  await writeSecrets(secrets);
}
