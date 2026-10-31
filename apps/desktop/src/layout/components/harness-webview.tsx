import type { ComponentType, CSSProperties, RefObject } from 'react'
import { useEventListener } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useStore } from 'valtio-define'
import { useListen } from '@/hooks/use-listen'
import { store } from '@/store'
import { zoomActionFromShortcut } from '@/utils/zoom'
import { Loadable } from './loadable'

export interface DshWebviewElement extends HTMLElement {
  setZoomFactor: (factor: number) => void
  executeJavaScript: <T>(script: string) => Promise<T>
}

interface NativeWebviewProps {
  'ref': RefObject<DshWebviewElement | null>
  'src': string
  'title': string
  'style'?: CSSProperties
  'data-testid': string
}

const NativeWebview = 'webview' as unknown as ComponentType<NativeWebviewProps>

export interface HarnessWebviewProps {
  srcOverride?: string | null
  borderTint?: string | null
}

export function HarnessWebview({ srcOverride = null, borderTint = null }: HarnessWebviewProps) {
  const webviewRef = useRef<DshWebviewElement>(null)
  const documentReadyRef = useRef(false)
  const lifecycleRef = useRef(Promise.resolve())
  const harness = useStore(store.harness)
  const setting = useStore(store.setting)
  const zoomFactorRef = useRef(setting.zoom_factor)
  zoomFactorRef.current = setting.zoom_factor
  const { t } = useTranslation()
  const remoteMode = srcOverride !== null && srcOverride !== ''
  const mounted = remoteMode || harness.serviceHealthy
  const src = remoteMode ? srcOverride : harness.webviewSrc
  const generation = harness.webviewKey
  const requestKey = `${src}#${generation}`
  const [prepared, setPrepared] = useState<{ key: string, label: string } | null>(null)
  const [preparationError, setPreparationError] = useState('')
  const guest = prepared?.key === requestKey ? prepared : null

  useEffect(() => {
    if (!mounted || !src)
      return
    let disposed = false
    let activeLabel: string | undefined
    setPreparationError('')
    lifecycleRef.current = lifecycleRef.current.then(async () => {
      if (disposed)
        return
      const label = await invoke<string>('prepare_harness_webview', { url: src, navigation: (window as Window & { __DSH_NAVIGATION__?: unknown }).__DSH_NAVIGATION__ ?? null })
      activeLabel = label
      if (!disposed)
        setPrepared({ key: requestKey, label })
    }).catch((error) => {
      if (!disposed)
        setPreparationError(String(error))
    })
    return () => {
      disposed = true
      setPrepared(null)
      lifecycleRef.current = lifecycleRef.current.then(async () => {
        if (activeLabel)
          await invoke('release_harness_webview', { label: activeLabel })
      }).catch(error => console.error('[Webview] failed to release guest:', error))
    }
  }, [mounted, src, requestKey])

  useEffect(() => {
    const webview = webviewRef.current
    if (!webview || !guest)
      return
    let disposed = false
    let documentVersion = 0
    const current = () => !disposed && store.harness.webviewKey === generation
    documentReadyRef.current = false
    const ready = () => {
      documentReadyRef.current = true
      webview.setZoomFactor(zoomFactorRef.current)
    }
    const loaded = () => {
      if (remoteMode || !current())
        return
      store.harness.markWebviewLoaded()
      const version = ++documentVersion
      void webview.executeJavaScript<boolean>(`Boolean(window.location.origin === ${JSON.stringify(new URL(src).origin)} && Array.isArray(window.__DSH_BOOT__?.entries) && window.__TAURI_INTERNALS__?.metadata?.currentWebview?.label === ${JSON.stringify(guest.label)})`)
        .then((alive) => {
          if (alive && current() && version === documentVersion)
            store.harness.markWebviewAlive(generation)
        })
        .catch(error => console.warn('[Webview] failed to verify guest document:', error))
    }
    const leaving = (event: Event) => {
      const { isMainFrame, isInPlace } = event as Event & { isMainFrame?: boolean, isInPlace?: boolean }
      if (isMainFrame !== true || isInPlace || remoteMode || !current())
        return
      documentVersion++
      documentReadyRef.current = false
      store.harness.markWebviewLeaving()
    }
    const crashed = () => {
      if (!remoteMode && current()) {
        documentVersion++
        documentReadyRef.current = false
        store.harness.markWebviewLeaving()
        store.harness.markWebviewError()
      }
    }
    const failed = (event: Event) => {
      const { errorCode, isMainFrame } = event as Event & { errorCode?: number, isMainFrame?: boolean }
      if (!remoteMode && current() && errorCode !== -3 && isMainFrame !== false)
        crashed()
    }
    webview.addEventListener('dom-ready', ready)
    webview.addEventListener('did-finish-load', loaded)
    webview.addEventListener('did-fail-load', failed)
    webview.addEventListener('did-start-navigation', leaving)
    webview.addEventListener('render-process-gone', crashed)
    return () => {
      disposed = true
      documentReadyRef.current = false
      webview.removeEventListener('dom-ready', ready)
      webview.removeEventListener('did-finish-load', loaded)
      webview.removeEventListener('did-fail-load', failed)
      webview.removeEventListener('did-start-navigation', leaving)
      webview.removeEventListener('render-process-gone', crashed)
    }
  }, [guest, src, generation, remoteMode])

  useEffect(() => {
    if (documentReadyRef.current)
      webviewRef.current?.setZoomFactor(setting.zoom_factor)
  }, [setting.zoom_factor])

  function zoom(action: 'increase' | 'decrease' | 'reset') {
    void store.setting.zoom(action).catch(error => console.error('[Zoom] failed to save zoom:', error))
  }
  useEventListener('keydown', (event: KeyboardEvent) => {
    const action = zoomActionFromShortcut(event)
    if (action) {
      event.preventDefault()
      zoom(action)
    }
  }, { capture: true })
  useListen<{ action?: string }>('dsh://zoom-shortcut', ({ payload }) => {
    if (payload.action === 'increase' || payload.action === 'decrease' || payload.action === 'reset')
      zoom(payload.action)
  })
  useListen('dsh://appearance:ready', () => {
    if (!remoteMode) {
      store.harness.markWebviewBootReady()
    }
  })

  if (preparationError)
    return <Loadable errorMsg={preparationError} onRetry={() => store.harness.refreshWebview()} />
  if (!mounted || !guest)
    return <Loadable subtitle={t(harness.startupStatusKey)} />

  return (
    <div className="relative flex min-h-0 flex-1" style={borderTint ? { borderColor: borderTint, borderWidth: 2, borderStyle: 'solid' } : undefined}>
      <NativeWebview
        key={guest.label}
        ref={webviewRef}
        data-testid="dsh-harness-webview"
        title="Harness"
        src={src}
        style={{ width: '100%', height: '100%', border: 0, visibility: !remoteMode && harness.showWebviewError ? 'hidden' : 'visible' }}
      />
      {!remoteMode && harness.showWebviewError && (
        <div className="absolute inset-0">
          <Loadable errorMsg={t('ui.webview_error')} onRetry={() => store.harness.refreshWebview()} />
        </div>
      )}
    </div>
  )
}
