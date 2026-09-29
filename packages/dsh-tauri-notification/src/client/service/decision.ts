import type { NotificationKind, NotificationSettings, TurnCompleteMode } from '../types'

/** 轮次完成通知的触发条件。 */
export interface TurnGateInput {
  /** 用户设置：从不 / 仅在未聚焦时 / 始终。 */
  readonly mode: TurnCompleteMode
  /**
   * 宿主窗口是否隐藏/最小化/失焦。
   *
   * iframe 内的 `document.visibilityState` 已由宿主窗口补丁（`NOTIFICATION_SHIM_JS`）
   * 映射到宿主窗口状态，因此这里直接用标准 API 即可。
   */
  readonly hostHidden: boolean
  /** 产生通知的会话。 */
  readonly sessionId: string
  /** 当前正在查看的会话。 */
  readonly currentSessionId: string | undefined
}

/**
 * 轮次完成是否该提醒。
 *
 * `background` 的语义与 ChatGPT 面板一致：窗口不在前台，**或**用户已切到别的会话，
 * 都算「未聚焦」。
 *
 * 当前会话读不到时（会话投影缺席 / 还没打开任何会话）按「未聚焦」处理：参考实现
 * `source/dsh-notification/src/client/notifier.ts` 的 `shouldShow` 也只在能确定
 * 「用户就停在这个会话上」时才抑制，宁可多提醒一次也不能漏掉一次。
 */
export function allowTurnNotification(input: TurnGateInput): boolean {
  if (input.mode === 'never')
    return false
  if (input.mode === 'always')
    return true
  if (input.hostHidden)
    return true
  return input.currentSessionId !== input.sessionId
}

/** 权限 / 提问通知各自的开关。 */
export function allowPendingNotification(settings: NotificationSettings, kind: 'approval' | 'question'): boolean {
  return kind === 'approval' ? settings.approval : settings.question
}

/**
 * 通知 tag。
 *
 * 必须命中宿主补丁的 `sessionIdFromTag`
 * （`/^dsh-notification-(?:pending-)?(.+)-\d+$/`），点击回传时才能反解出 sessionId。
 */
export function notificationTag(sessionId: string, kind: NotificationKind, sequence: number): string {
  const prefix = kind === 'turn' ? 'dsh-notification' : 'dsh-notification-pending'
  return `${prefix}-${sessionId}-${sequence}`
}

/** 截断正文，避免超长标题/命令撑爆系统通知。 */
export function boundText(text: string, maxChars: number): string {
  const trimmed = text.trim()
  return trimmed.length <= maxChars ? trimmed : `${trimmed.slice(0, maxChars - 1)}…`
}
