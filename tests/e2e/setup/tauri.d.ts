interface TauriInternals {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>
  metadata?: { currentWindow?: { label?: string } }
}

declare global {
  interface Window {
    __TAURI_INTERNALS__: TauriInternals
  }
}

export {}
