import type { EventHandlerRequest } from 'dsh-tauri'
import type { SessionResumeResponse } from '../../../types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { session } from '../../../service/session'

export default defineEventHandler<EventHandlerRequest, Promise<SessionResumeResponse>>(async (event) => {
  const body = (await readBody<{ sessionId?: string, recoverFromSessionId?: string }>(event)) ?? {}
  const sessionId = typeof body.sessionId === 'string' ? body.sessionId.trim() : ''
  if (sessionId.length === 0) {
    event.res.status = 400
    return { error: '缺少 sessionId' }
  }
  const recoverFromSessionId = typeof body.recoverFromSessionId === 'string' && body.recoverFromSessionId.trim().length > 0
    ? body.recoverFromSessionId.trim()
    : undefined
  const outcome = await session.resume(sessionId, recoverFromSessionId)
  if (outcome.ok)
    return { ok: true }
  event.res.status = outcome.code
  return { error: outcome.error }
})
