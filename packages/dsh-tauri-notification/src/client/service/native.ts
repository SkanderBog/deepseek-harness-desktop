import type { NativeNotificationInput } from '../types'

/** 宿主窗口补丁读取的 actions 项（`{ action, title }`，见 `NOTIFICATION_SHIM_JS`）。 */
interface ShimAction {
  action: string
  title: string
}

interface ShimNotificationOptions {
  body?: string
  tag?: string
  sessionId?: string
  requireInteraction?: boolean
  actions?: ShimAction[]
}

/** 宿主补丁替换过的 Notification 额外带回调属性（标准 Notification 没有 `onaction`）。 */
interface ShimNotification {
  onclick: ((event: Event) => void) | null
  onaction?: ((event: { action?: string }) => void) | null
}

/**
 * 弹一条原生通知，并接住它的点击与按钮回调。
 *
 * 不直接 `invoke` Tauri 命令，而是走宿主窗口注入的 `window.Notification` 补丁：
 * 补丁把构造参数 postMessage 给 Rust 侧 `show_native_notification`，等原生通知被
 * 点击（`onclick`）或按下按钮（`onaction`）时再回调本对象——这是当前唯一能拿到
 * 按钮 action 的通路。
 */
export function showNativeNotification(input: NativeNotificationInput): void {
  if (typeof Notification === 'undefined')
    return
  const options: ShimNotificationOptions = {
    body: input.body,
    tag: input.tag,
    sessionId: input.sessionId,
    requireInteraction: input.requireInteraction ?? false,
  }
  if (input.actions && input.actions.length > 0)
    options.actions = input.actions.map(action => ({ action: action.id, title: action.title }))
  const notification = new Notification(input.title, options as unknown as NotificationOptions) as Notification & ShimNotification
  notification.onclick = () => {
    input.onClick?.()
  }
  if (input.onAction) {
    notification.onaction = (event) => {
      input.onAction?.(String(event?.action ?? ''))
    }
  }
}
