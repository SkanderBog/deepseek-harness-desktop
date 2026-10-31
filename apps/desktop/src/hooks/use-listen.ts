import type { EventCallback, Options, UnlistenFn } from '@tauri-apps/api/event'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useRef } from 'react'

export function useListen<T>(event: string, handler: EventCallback<T>, options?: Options): void {
  const handlerRef = useRef(handler)
  const optionsRef = useRef(options)
  handlerRef.current = handler
  optionsRef.current = options

  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    let disposed = false

    const options = optionsRef.current
    void listen<T>(event, payload => handlerRef.current(payload),
      event.startsWith('dsh://') && options?.target === undefined
        ? { ...options, target: { kind: 'Window', label: getCurrentWindow().label } }
        : options)
      .then((fn) => {
        if (disposed)
          fn()
        else
          unlisten = fn
      })
      .catch(err => console.error(`[useListen] failed to listen ${event}:`, err))

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [event])
}
