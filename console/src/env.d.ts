/// <reference types="vite/client" />

/** Build-time constant (vite.config.ts `define`): true only for `--mode mock` builds. */
declare const __YUNORI_MOCK__: boolean;

interface ImportMetaEnv {
  readonly VITE_MOCK?: string;
}
