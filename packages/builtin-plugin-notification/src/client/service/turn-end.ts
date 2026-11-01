import { getTurnEnd } from '../apis'

/**
 * 用户主动中断会话时 `turn/end` 的 `reason.kind`。
 *
 * 内核把手动中断标成 `aborted`（`completed` 才是正常跑完）；`interrupted` 是同类语义的
 * 历史别名，一并认掉：两者都不该弹「回合完成」通知。
 */
const USER_INTERRUPT_REASONS: ReadonlySet<string> = new Set(['aborted', 'interrupted', 'archived'])

export async function readTurnEnd(sessionId: string): Promise<{ interrupted: boolean, turn?: number }> {
  if (sessionId.length === 0)
    return { interrupted: false }
  try {
    const fact = await getTurnEnd({ sessionId })
    return {
      interrupted: typeof fact?.reason === 'string' && USER_INTERRUPT_REASONS.has(fact.reason),
      turn: typeof fact?.turn === 'number' && fact.turn >= 0 ? fact.turn : undefined,
    }
  }
  catch {
    return { interrupted: false }
  }
}
