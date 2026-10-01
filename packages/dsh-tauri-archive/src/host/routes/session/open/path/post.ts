import type { EventHandlerRequest } from 'dsh-tauri'
import type { OpenSessionDirResult, SessionIdBody } from '../../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { session } from '../../../../service/session'
import { validateSessionId } from '../../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<OpenSessionDirResult>>(async (event) => {
  const body = await readBody<SessionIdBody>(event, { type: 'json' })
  const sessionId = validateSessionId(event, body)
  if (typeof sessionId !== 'string')
    return sessionId

  const result = await session.openDir(sessionId)
  if (!result.ok)
    event.res.status = 400
  return result
})
