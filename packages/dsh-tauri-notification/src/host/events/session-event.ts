import { recordTurnEnd } from '../service/turn-end'

/**
 * `session/event` 订阅：只挑 `turn/end` 记下结束原因。
 *
 * 事件回调签名是 `(session, event)`（与 `dsh-tauri-worktree`、`dsh-tauri-pet` 一致），
 * 会话 id 取 `session.id`。原因缺失时直接放弃：客户端读不到事实就退化成「照常通知」，
 * 不会因为宿主事件形状变化而漏掉通知。
 */
export function handleSessionEvent(session: any, event: any): void {
  if (event?.type !== 'turn/end')
    return
  const sessionId = session?.id
  if (typeof sessionId !== 'string' || sessionId.length === 0)
    return
  const reason = event?.data?.reason?.kind
  if (typeof reason !== 'string' || reason.length === 0)
    return
  const turn = event?.data?.turn
  recordTurnEnd(sessionId, { reason, turn: typeof turn === 'number' ? turn : -1 })
}
