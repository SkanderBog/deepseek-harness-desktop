import type { PluginListener } from '@tauri-apps/api/core'
import { onAction } from '@choochmeque/tauri-plugin-notifications-api'
import { useEffect, useRef } from 'react'

export interface NotificationExtra {
  sessionId?: string
  title?: string
  tag?: string
}

export interface NotificationActionEvent {
  actionId?: string
  inputValue?: string | null
  extra?: NotificationExtra | null
  notification?: { extra?: NotificationExtra } | null
}

export function useNotificationAction(handler: (event: NotificationActionEvent) => void): void {
  const handlerRef = useRef(handler)
  handlerRef.current = handler

  useEffect(() => {
    let listener: PluginListener | undefined
    let disposed = false

    void onAction((payload) => {
      handlerRef.current((payload as unknown as NotificationActionEvent | null) ?? {})
    }).then((unlisten) => {
      if (disposed)
        void unlisten.unregister()
      else
        listener = unlisten
    }).catch(error => console.error('[useNotificationAction] failed to subscribe:', error))

    return () => {
      disposed = true
      void listener?.unregister()
    }
  }, [])
}
