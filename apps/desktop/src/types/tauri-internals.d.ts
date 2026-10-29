import type { invoke } from '@tauri-apps/api/core'

declare global {
  interface Window {
    __TAURI_INTERNALS__: { invoke: typeof invoke }
  }
}
