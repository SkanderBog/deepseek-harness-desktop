import type { PendingInteractionFace, SessionStatusFace, UiSessionFace } from '../types'

export interface DecideResult {
  ok: boolean
  error?: string
}

/** 读取某会话的当前状态；没有 `uiSession` 服务或会话已不在快照里时为 `undefined`。 */
export function readSessionStatus(uiSession: UiSessionFace | undefined, sessionId: string): SessionStatusFace | undefined {
  return uiSession?.sessionStatus.getSnapshot().get(sessionId)
}

/** 读取某会话当前**最高优先级**的待处理交互（官方语义：每个会话只有一个）。 */
export function readPendingInteraction(uiSession: UiSessionFace | undefined, sessionId: string): PendingInteractionFace | undefined {
  return readSessionStatus(uiSession, sessionId)?.pendingInteraction
}

/** 把官方 kind 收敛成通知类别；未知交互不弹通知。 */
export function pendingNotificationKind(pending: PendingInteractionFace | undefined): 'approval' | 'question' | undefined {
  if (!pending)
    return undefined
  if (pending.kind === 'approval')
    return 'approval'
  if (pending.kind === 'question' || pending.kind === 'plan-review')
    return 'question'
  return undefined
}

/** 待处理交互的稳定标识，用于去重（同一次挂起只通知一次）。 */
export function pendingIdentity(pending: PendingInteractionFace): string {
  return `${pending.kind}:${pending.key}`
}

/** 交互正文：授权取「工具名: 原因」，提问取首个问题。 */
export function pendingDetail(pending: PendingInteractionFace): string {
  if (pending.kind === 'approval') {
    const reason = resolvedReason(pending)
    if (pending.toolName && reason)
      return `${pending.toolName}: ${reason}`
    return pending.toolName ?? reason
  }
  const first = pending.questions?.[0]
  return first?.question ?? first?.header ?? pending.toolName ?? ''
}

function resolvedReason(pending: PendingInteractionFace): string {
  const display = pending.displayReason
  if (typeof display === 'string')
    return display
  if (display && typeof display === 'object') {
    const dict = display as Record<string, string>
    return dict.zh ?? dict.en ?? Object.values(dict)[0] ?? ''
  }
  return pending.reason ?? ''
}

/**
 * 通过官方待处理交互回答授权——通知上的「批准」按钮最终走这里。
 *
 * 只有同一次挂起期间注册过、且仍可回答的交互才能被外部回答；否则返回失败
 * （例如授权已在界面里处理过，或该交互已降级为不可回答）。
 */
export async function approveInteraction(uiSession: UiSessionFace | undefined, sessionId: string): Promise<DecideResult> {
  const pending = readPendingInteraction(uiSession, sessionId)
  if (!pending || pending.kind !== 'approval')
    return { ok: false, error: 'APPROVAL_UNAVAILABLE' }
  if (pending.answerable === false || typeof pending.answer !== 'function')
    return { ok: false, error: 'APPROVAL_NOT_ANSWERABLE' }
  try {
    await pending.answer('allowed-once')
    return { ok: true }
  }
  catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}
