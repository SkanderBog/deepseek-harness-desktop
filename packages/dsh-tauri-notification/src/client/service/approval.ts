import type { ApprovalDecision, PendingInteractionFace, SessionStatusFace, UiSessionFace } from '../types'
import { boundText } from './decision'

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
    const reason = conciseReason(resolvedReason(pending))
    if (pending.toolName && reason)
      return `${pending.toolName}: ${reason}`
    return pending.toolName ?? reason
  }
  const first = pending.questions?.[0]
  return first?.question ?? first?.header ?? pending.toolName ?? ''
}

/** 授权原因只保留到第一个分句分隔符（中文/英文冒号、句号、换行）。 */
const REASON_CLAUSE_DELIMITER = /[：:。\n]/
/** 授权原因长度上限：正文里还要放动作标签与工具名，原因留一行即可。 */
const MAX_REASON_CHARS = 60

/**
 * 授权原因只保留首个分句。
 *
 * `displayReason` 的后半段是给审计用的完整说明（例如「允许本次操作使用 x 权限：<沙箱拒绝
 * 详情>」），整段塞进系统通知会把真正有信息量的部分挤掉；界面里的授权卡片仍然显示全文。
 */
function conciseReason(reason: string): string {
  const clause = reason.split(REASON_CLAUSE_DELIMITER)[0]?.trim() ?? ''
  return boundText(clause || reason, MAX_REASON_CHARS)
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
  return decideApproval(uiSession, sessionId, 'allowed-once')
}

/** 通知上的「拒绝」按钮走这里，语义与界面里的拒绝一致。 */
export async function rejectInteraction(uiSession: UiSessionFace | undefined, sessionId: string): Promise<DecideResult> {
  return decideApproval(uiSession, sessionId, 'rejected')
}

async function decideApproval(uiSession: UiSessionFace | undefined, sessionId: string, decision: ApprovalDecision): Promise<DecideResult> {
  const pending = readPendingInteraction(uiSession, sessionId)
  if (!pending || pending.kind !== 'approval')
    return { ok: false, error: 'APPROVAL_UNAVAILABLE' }
  if (pending.answerable === false || typeof pending.answer !== 'function')
    return { ok: false, error: 'APPROVAL_NOT_ANSWERABLE' }
  try {
    await pending.answer(decision)
    return { ok: true }
  }
  catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * 通过官方待处理交互回答提问——通知里的输入框文本走这里。
 *
 * 系统通知渲染不了选项列表，所以把用户输入的自由文本当作官方的 `custom`（「其他」）
 * 答案回传，`selected` 留空；问题 id 取批次里的第一个问题（官方每个会话同一时刻只有
 * 一个待处理交互，通知也只展示它的首个问题）。
 */
export async function answerQuestion(uiSession: UiSessionFace | undefined, sessionId: string, text: string): Promise<DecideResult> {
  const pending = readPendingInteraction(uiSession, sessionId)
  if (!pending || (pending.kind !== 'question' && pending.kind !== 'plan-review'))
    return { ok: false, error: 'QUESTION_UNAVAILABLE' }
  if (pending.answerable === false || typeof pending.answer !== 'function')
    return { ok: false, error: 'QUESTION_NOT_ANSWERABLE' }
  const custom = text.trim()
  if (!custom)
    return { ok: false, error: 'QUESTION_EMPTY' }
  const id = pending.questions?.[0]?.id
  if (!id)
    return { ok: false, error: 'QUESTION_ID_MISSING' }
  try {
    await pending.answer({ answers: [{ id, selected: [], custom }] })
    return { ok: true }
  }
  catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}
