/// <reference types="vite/client" />

declare module "*.svg" {
  const src: string;
  export default src;
}

interface ImportMetaEnv {
  readonly VITE_WS_URL?: string;
  readonly VITE_AUTH_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

interface LitecodeDebugApi {
  on: (spec?: string) => void;
  off: () => void;
  status: () => string | null;
}

interface Window {
  litecodeDebug?: LitecodeDebugApi;
}
