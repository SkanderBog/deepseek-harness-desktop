import type { RefObject } from 'react'
import { useWatch } from '@reause/core'
import { Effect, EffectState, getCurrentWindow } from '@tauri-apps/api/window'
import { type } from '@tauri-apps/plugin-os'
import { useRef } from 'react'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { appearanceBackdropFilter, appearanceColors, normalizeAppearance } from '../../packages/dsh-tauri/src/shared/appearance'
import { useDshStyle } from './use-dsh-style'
import { useIframeMessage } from './use-iframe-message'
import { useIframePost } from './use-iframe-post'

export function useAppearance(iframeRef: RefObject<HTMLIFrameElement | null>) {
  const { appearance } = useStore(store.setting)
  const [dshStyle] = useDshStyle()
  const post = useIframePost(iframeRef)
  const readyRef = useRef(false)
  const nativeEffectQueueRef = useRef(Promise.resolve())
  const transparent = (window as Window & { __DSH_TRANSPARENT__?: boolean }).__DSH_TRANSPARENT__ === true

  function sendAppearance() {
    const value = normalizeAppearance(appearance)
    post({ type: 'dsh://appearance', appearance: { ...value, transparency: transparent && value.transparency, opacity: transparent && value.transparency ? value.opacity : 100 } })
  }

  useIframeMessage<{ type?: string }>(iframeRef, (message) => {
    if (message.type === 'dsh://appearance:ready') {
      readyRef.current = true
      sendAppearance()
    }
    else if (message.type === 'dsh://plugin-boot:leaving') {
      readyRef.current = false
    }
  })
  useWatch([appearance, dshStyle.colorScheme], () => {
    if (readyRef.current)
      sendAppearance()
  }, { immediate: true })
  useWatch(appearance, () => {
    let platform: ReturnType<typeof type>
    try {
      platform = type()
    }
    catch {
      return
    }
    if (!transparent || (platform !== 'windows' && platform !== 'macos'))
      return

    const enabled = appearanceBackdropFilter(normalizeAppearance(appearance)) !== 'none'
    const appWindow = getCurrentWindow()
    nativeEffectQueueRef.current = nativeEffectQueueRef.current
      .then(() => enabled
        ? appWindow.setEffects({
            effects: [Effect.Acrylic, Effect.Mica, Effect.UnderWindowBackground],
            state: EffectState.FollowsWindowActiveState,
          })
        : appWindow.clearEffects())
      .catch(error => console.warn('[useAppearance] native backdrop effect failed:', error))
  }, { immediate: true })
  const value = normalizeAppearance(appearance)
  const { canvas, panel, surface, text, muted, accent, border } = appearanceColors(value, dshStyle.colorScheme ?? (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark'))
  const alpha = transparent && value.transparency ? value.opacity : 100
  const backdropFilter = transparent ? appearanceBackdropFilter(value) : 'none'
  return value.palette === 'default' && alpha === 100
    ? ''
    : `
      ${alpha < 100 ? 'html,body{background:transparent!important}' : ''}
      html[data-theme]{--color-canvas:${canvas};--color-panel:${panel};--color-panel-2:${surface};--color-ink:${text};--color-info:${accent};--foreground:${text};--muted:${muted};--background:${canvas};--surface:${panel};--surface-secondary:${surface};--surface-tertiary:${surface};${border ? `--color-line:${border};--color-line-strong:${border};--color-btn-border:${border};--border:${border};--separator:${border};--field-border:${border}` : ''}}
      [data-testid="dsh-navbar-root"]{background:color-mix(in srgb,${panel} ${alpha}%,transparent)!important;${backdropFilter === 'none' ? '' : `backdrop-filter:${backdropFilter};-webkit-backdrop-filter:${backdropFilter};`}}
      [data-testid="dsh-shell-root"]>main{background:transparent!important}
    `
}
