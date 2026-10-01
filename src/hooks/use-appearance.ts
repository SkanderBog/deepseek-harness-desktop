import type { RefObject } from 'react'
import { useWatch } from '@reause/core'
import { useRef } from 'react'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { appearanceColors, normalizeAppearance } from '../../packages/dsh-tauri/src/shared/appearance'
import { useDshStyle } from './use-dsh-style'
import { useIframeMessage } from './use-iframe-message'
import { useIframePost } from './use-iframe-post'

export function useAppearance(iframeRef: RefObject<HTMLIFrameElement | null>) {
  const { appearance } = useStore(store.setting)
  const [dshStyle] = useDshStyle()
  const post = useIframePost(iframeRef)
  const readyRef = useRef(false)
  const transparent = (window as Window & { __DSH_TRANSPARENT__?: boolean }).__DSH_TRANSPARENT__ === true

  function sendAppearance() {
    const value = normalizeAppearance(appearance)
    post({ type: 'dsh://appearance', appearance: { ...value, opacity: transparent ? value.opacity : 100 } })
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
  const value = normalizeAppearance(appearance)
  const { canvas, panel, surface, text, muted, accent } = appearanceColors(value, dshStyle.colorScheme ?? (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark'))
  const alpha = transparent ? value.opacity : 100
  return value.palette === 'default' && alpha === 100
    ? ''
    : `
      ${alpha < 100 ? 'html,body{background:transparent!important}' : ''}
      html[data-theme]{--color-canvas:${canvas};--color-panel:${panel};--color-panel-2:${surface};--color-ink:${text};--color-info:${accent};--foreground:${text};--muted:${muted};--background:${canvas};--surface:${panel};--surface-secondary:${surface};--surface-tertiary:${surface}}
      [data-testid="dsh-navbar-root"]{background:color-mix(in srgb,${panel} ${alpha}%,transparent)!important}
      [data-testid="dsh-shell-root"]>main{background:transparent!important}
    `
}
