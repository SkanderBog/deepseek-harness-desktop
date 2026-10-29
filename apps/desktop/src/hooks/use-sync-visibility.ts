/* eslint-disable react/exhaustive-deps */
import type { RefObject } from 'react'
import { useIntervalFn } from '@reause/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect } from 'react'

const VISIBILITY_POLL_INTERVAL = 5000
const appWindow = getCurrentWindow()

export interface VisibilityGuest {
  executeJavaScript: (code: string) => Promise<unknown>
}

// keep:effect 宿主窗口状态只能由原生窗口事件 + 兜底轮询驱动，卸载时必须注销。
export function useSyncVisibility(ref: RefObject<VisibilityGuest | null>): void {
  function dispatch(hidden: boolean) {
    const guest = ref.current
    if (!guest)
      return
    guest
      .executeJavaScript(`window.postMessage({ type: 'dsh://visibility-state', hidden: ${hidden} }, '*')`)
      .catch((error) => {
        console.error('[notification] sync visibility failed:', error)
      })
  }

  function syncVisibility() {
    void (async () => {
      try {
        const [minimized, visible, focused] = await Promise.all([
          appWindow.isMinimized(),
          appWindow.isVisible(),
          appWindow.isFocused(),
        ])
        // 未聚焦也必须算后台：用户切去别的应用时只看 visible 会误判成前台。
        dispatch(minimized || !visible || !focused)
      }
      catch (error) {
        console.error('[notification] sync visibility failed:', error)
      }
    })()
  }

  useEffect(() => {
    let disposed = false
    let unlisteners: Array<() => void> = []
    void (async () => {
      try {
        syncVisibility()
        const unFocus = await appWindow.onFocusChanged(() => {
          syncVisibility()
        })
        const unResized = await appWindow.onResized(() => {
          syncVisibility()
        })
        if (disposed) {
          unFocus()
          unResized()
        }
        else {
          unlisteners = [unFocus, unResized]
        }
      }
      catch (error) {
        console.error('[notification] visibility listeners failed:', error)
      }
    })()
    return () => {
      disposed = true
      unlisteners.forEach(fn => fn())
    }
  }, [])

  useIntervalFn(syncVisibility, VISIBILITY_POLL_INTERVAL)
}
