import type { RefObject } from 'react'
import { useWatch } from '@reause/core'
import { Effect, EffectState, getCurrentWindow } from '@tauri-apps/api/window'
import { type } from '@tauri-apps/plugin-os'
import { useRef } from 'react'
import { useStore } from 'valtio-define'
import { store } from '@/store'
import { appearanceBackdropFilter, appearanceBootCss, appearanceColors, appearanceSidebarFill, appearanceStartupFill, normalizeAppearance } from '../../packages/dsh-tauri/src/shared/appearance'
import { useDshStyle } from './use-dsh-style'
import { useIframeMessage } from './use-iframe-message'
import { useIframePost } from './use-iframe-post'

/**
 * 启动期外观握手（宿主侧）。
 *
 * 内嵌 dsh 的 boot 页（HARNESS + Loading plugins…）由内核在插件加载之前绘出，此刻
 * 外观插件尚未激活，透明只能由宿主在 document-start 就下发（帧内接收器见
 * `src-tauri/src/desktop/appearance.js.inc`）。宿主侧维护「当前代文档是否已确认收到」
 * 这一比特：boot CSS 的生成留在 React 里（调色板只有一份），帧内只负责原样落盘。
 */
const ACK_TTL_MS = 4000

let bootAppearance: { css: string, ack: string | null, timer: ReturnType<typeof setTimeout> | null } = { css: '', ack: null, timer: null }

function aliveAck(): boolean {
  return bootAppearance.ack !== null && Date.now() - Number(bootAppearance.ack) < ACK_TTL_MS
}

function acknowledge() {
  bootAppearance.ack = String(Date.now())
  if (bootAppearance.timer !== null) {
    clearTimeout(bootAppearance.timer)
    bootAppearance.timer = null
  }
}

/**
 * 标记「当前这一代 iframe 文档需要重新握手」：重载、换 URL、换远端隧道都会走到这里。
 * 旧文档的确认不能替新文档背书。
 */
export function resetBootAppearance() {
  bootAppearance.ack = null
  if (bootAppearance.timer !== null) {
    clearTimeout(bootAppearance.timer)
    bootAppearance.timer = null
  }
}

export function bootAppearancePending(): boolean {
  return !aliveAck()
}

/**
 * 生成启动期样式并登记最新一份。**不在这里下发给已挂载的 iframe**：外观投影挂在
 * Webview（启动阶段 iframe 尚未存在），下发时机由 Iframe 自己按「src 落定 + 本次
 * boot CSS」的 watch 负责；帧内 document-start 的主动请求也走同一条消息。
 */
export function createAppearanceBootCss(css: string) {
  const changed = bootAppearance.css !== css
  bootAppearance = { ...bootAppearance, css, ack: aliveAck() ? bootAppearance.ack : null }
  return changed
}

/** 已登记的最新一份启动期样式（宿主侧唯一投影来源，帧内只负责原样落盘）。 */
export function bootAppearanceCss(): string {
  return bootAppearance.css
}

export function useAppearance(iframeRef: RefObject<HTMLIFrameElement | null>) {
  const { appearance, hydrated } = useStore(store.setting)
  const [dshStyle] = useDshStyle()
  const post = useIframePost(iframeRef)
  const readyRef = useRef(false)
  const nativeEffectQueueRef = useRef(Promise.resolve())
  const transparent = (window as Window & { __DSH_TRANSPARENT__?: boolean }).__DSH_TRANSPARENT__ === true
  const value = normalizeAppearance(appearance)
  const payload = { ...value, transparency: transparent && value.transparency, opacity: transparent && value.transparency ? value.opacity : 100 }
  const bootCss = appearanceBootCss(payload)
  const scheme = dshStyle.colorScheme ?? (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark')

  function sendAppearance() {
    post({ type: 'dsh://appearance', appearance: payload, bootCss: bootAppearanceCss() })
  }

  // 帧内在 document-start 就请求一次（那时壳层可能还没挂载监听器），同时也在应用
  // 激活后按既有协议自报 ready；两条通路都回同一份投影，重复下发由帧内幂等处理。
  useIframeMessage<{ type?: string }>(iframeRef, (message) => {
    if (message.type === 'dsh://appearance:ready' || message.type === 'dsh://appearance:request' || message.type === 'dsh://plugin-boot:frame') {
      readyRef.current = true
      sendAppearance()
    }
    else if (message.type === 'dsh://plugin-boot:leaving') {
      readyRef.current = false
    }
    else if (message.type === 'dsh://appearance:applied') {
      acknowledge()
    }
  })
  useWatch([appearance, dshStyle.colorScheme], () => {
    if (readyRef.current)
      sendAppearance()
  }, { immediate: true })
  useWatch(bootCss, () => {
    createAppearanceBootCss(bootCss)
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
  const backdropFilter = transparent ? appearanceBackdropFilter(value) : 'none'
  if (value.palette === 'default' && alpha === 100 && !transparent)
    return ''
  const { canvas, panel, surface, text, muted, accent, border } = appearanceColors(value, scheme)
  return [
    alpha < 100 ? 'html,body{background:transparent!important}' : '',
    `html[data-theme]{--color-canvas:${canvas};--color-startup:${appearanceStartupFill(value, canvas)};--color-panel:${panel};--color-panel-2:${surface};--color-ink:${text};--color-info:${accent};--foreground:${text};--muted:${muted};--background:${canvas};--surface:${panel};--surface-secondary:${surface};--surface-tertiary:${surface};${border ? `--color-line:${border};--color-line-strong:${border};--color-btn-border:${border};--border:${border};--separator:${border};--field-border:${border};` : ''}}`,
    `[data-testid="dsh-navbar-root"]{background:${appearanceSidebarFill(canvas, panel, alpha < 100, alpha)}!important;${backdropFilter === 'none' ? '' : `backdrop-filter:${backdropFilter};-webkit-backdrop-filter:${backdropFilter};`}}`,
    alpha < 100 && value.sidebarOnly ? `[data-testid="dsh-config-dialog"]{background:${panel}!important}` : '',
    // 启动页（Loadable / 全屏恢复页 / 预装引导）与内容区一起透明：启动页不属于内容区，
    // 但必须跟 navbar、侧边栏用同一层 alpha，否则窗口背后只透出一半。
    '[data-testid="dsh-shell-root"]>main{background:transparent!important}',
  ].filter(Boolean).join('\n')
}
