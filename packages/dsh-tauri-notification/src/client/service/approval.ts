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
 *
 * `expectedIdentity` 是这条通知**当初展示**的那次挂起（`pendingIdentity`）。系统通知会在
 * 心里一直挂着直到用户处理它，所以一条过期的「批准」可能在原授权早已被界面处理、同一会话
 * 又冒出**新的**授权之后才被点到；身份对不上就拒绝，绝不回答「当前恰好挂着的那一个」。
 */
export async function approveInteraction(uiSession: UiSessionFace | undefined, sessionId: string, expectedIdentity?: string): Promise<DecideResult> {
  return decideApproval(uiSession, sessionId, 'allowed-once', expectedIdentity)
}

/** 通知上的「拒绝」按钮走这里，语义与界面里的拒绝一致。 */
export async function rejectInteraction(uiSession: UiSessionFace | undefined, sessionId: string, expectedIdentity?: string): Promise<DecideResult> {
  return decideApproval(uiSession, sessionId, 'rejected', expectedIdentity)
}

async function decideApproval(uiSession: UiSessionFace | undefined, sessionId: string, decision: ApprovalDecision, expectedIdentity?: string): Promise<DecideResult> {
  const pending = readPendingInteraction(uiSession, sessionId)
  if (!pending || pending.kind !== 'approval')
    return { ok: false, error: 'APPROVAL_UNAVAILABLE' }
  if (expectedIdentity !== undefined && pendingIdentity(pending) !== expectedIdentity)
    return { ok: false, error: 'APPROVAL_STALE' }
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
 * 通知里那个输入框能回答的问题 id；**恰好一个问题**时才有值。
 *
 * 通知只有一个文本框，而官方契约要求每个问题各带一条答案：多问题批次从这里只答第一个，
 * 会让宿主把整批当成已答完。所以这类批次不提供回复按钮（交给会话界面），此函数也是
 * 「要不要给回复按钮」的唯一判据。
 */
export function answerableQuestionId(pending: PendingInteractionFace): string | undefined {
  const questions = pending.questions ?? []
  if (questions.length !== 1)
    return undefined
  const id = questions[0]?.id
  return typeof id === 'string' && id.length > 0 ? id : undefined
}

/**
 * 通过官方待处理交互回答提问——通知里的输入框文本走这里。
 *
 * 系统通知渲染不了选项列表，所以把用户输入的自由文本当作官方的 `custom`（「其他」）
 * 答案回传，`selected` 留空。
 */
export async function answerQuestion(uiSession: UiSessionFace | undefined, sessionId: string, text: string, expectedIdentity?: string): Promise<DecideResult> {
  const pending = readPendingInteraction(uiSession, sessionId)
  if (!pending || (pending.kind !== 'question' && pending.kind !== 'plan-review'))
    return { ok: false, error: 'QUESTION_UNAVAILABLE' }
  if (expectedIdentity !== undefined && pendingIdentity(pending) !== expectedIdentity)
    return { ok: false, error: 'QUESTION_STALE' }
  if (pending.answerable === false || typeof pending.answer !== 'function')
    return { ok: false, error: 'QUESTION_NOT_ANSWERABLE' }
  if ((pending.questions ?? []).length !== 1)
    return { ok: false, error: 'QUESTION_NOT_SINGLE' }
  const custom = text.trim()
  if (!custom)
    return { ok: false, error: 'QUESTION_EMPTY' }
  const id = answerableQuestionId(pending)
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
