import { useWatch } from '@reause/core'
import { emitTo } from '@tauri-apps/api/event'
import { type } from '@tauri-apps/plugin-os'
import { useRef } from 'react'
import { Effect, EffectState, getCurrentWindow } from 'tauri-runtime-electron/api/window'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { appearanceColors, appearanceSidebarFill, appearanceStartupFill, appearanceTranslucent, normalizeAppearance } from '../../../../packages/builtin-plugin-tauri/src/shared/appearance'
import { useDshStyle } from './use-dsh-style'
import { useListen } from './use-listen'

export function useAppearance() {
  const { appearance, hydrated } = useStore(store.setting)
  const [dshStyle] = useDshStyle()
  const readyRef = useRef(false)
  const nativeEffectQueueRef = useRef(Promise.resolve())
  const transparent = (window as Window & { __DSH_TRANSPARENT__?: boolean }).__DSH_TRANSPARENT__ === true
  const value = normalizeAppearance(appearance)
  const payload = { ...value, transparency: transparent && value.transparency, opacity: transparent && value.transparency ? value.opacity : 100 }
  const scheme = dshStyle.colorScheme ?? (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark')

  function sendAppearance() {
    void emitTo({ kind: 'Window', label: getCurrentWindow().label }, 'dsh://appearance', { appearance: payload })
  }

  const onReady = () => {
    readyRef.current = true
    sendAppearance()
  }
  useListen<void>('dsh://appearance:ready', onReady)
  useWatch([appearance, dshStyle.colorScheme], () => {
    if (readyRef.current)
      sendAppearance()
  }, { immediate: true })
  useWatch([appearance, hydrated], () => {
    if (!hydrated)
      return
    let platform: ReturnType<typeof type>
    try {
      platform = type()
    }
    catch {
      return
    }
    if (!transparent || (platform !== 'windows' && platform !== 'macos'))
      return

    const appWindow = getCurrentWindow()
    nativeEffectQueueRef.current = nativeEffectQueueRef.current
      .then(() => value.transparency && value.blur
        ? appWindow.setEffects({
            effects: [Effect.Acrylic, Effect.Mica, Effect.UnderWindowBackground],
            state: EffectState.FollowsWindowActiveState,
          })
        : appWindow.clearEffects())
      .catch(error => console.warn('[useAppearance] native backdrop effect failed:', error))
  }, { immediate: true })

  const alpha = transparent && value.transparency ? value.opacity : 100
  const backdropFilter = transparent && appearanceTranslucent(value) && value.blur ? 'blur(16px)' : 'none'
  if (value.palette === 'default' && alpha === 100 && !transparent)
    return ''
  const { canvas, panel, surface, text, muted, accent, border } = appearanceColors(value, scheme)
  return [
    alpha < 100 ? 'html,body,[data-testid="dsh-shell-root"]{background:transparent!important}' : '',
    `html[data-theme]{--color-canvas:${canvas};--color-startup:${appearanceStartupFill(value, canvas)};--color-panel:${panel};--color-panel-2:${surface};--color-ink:${text};--color-info:${accent};--foreground:${text};--muted:${muted};--background:${canvas};--surface:${panel};--surface-secondary:${surface};--surface-tertiary:${surface};${border ? `--color-line:${border};--color-line-strong:${border};--color-btn-border:${border};--border:${border};--separator:${border};--field-border:${border};` : ''}}`,
    `[data-testid="dsh-navbar-root"]{background:${appearanceSidebarFill(canvas, panel, alpha < 100, alpha)}!important;${backdropFilter === 'none' ? '' : `backdrop-filter:${backdropFilter};-webkit-backdrop-filter:${backdropFilter};`}}`,
    alpha < 100 && value.sidebarOnly ? `[data-testid="dsh-config-dialog"]{background:${panel}!important}` : '',
    '[data-testid="dsh-shell-root"]>main{background:transparent!important}',
  ].filter(Boolean).join('\n')
}
