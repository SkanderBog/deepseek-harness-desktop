import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshSessionRoleResponse } from '../../index.types'
import { defineEventHandler } from 'dsh-tauri'
import { session } from '../../../service/session'
import { guarded } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshSessionRoleResponse>>(async event =>
  guarded(event, async () => {
    const role = session.role()
    return { ...role, role: role.remote ? 'remote' : 'local' }
  }))
