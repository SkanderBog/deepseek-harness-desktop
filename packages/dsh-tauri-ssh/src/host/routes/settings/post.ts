import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshSettingsBody, SshSettingsResponse } from '../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../service/machine'
import { enabledOf, guarded } from '../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshSettingsResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshSettingsBody>(event)) ?? {}
    const enabled = enabledOf(body)
    await machine.setEnabled(enabled)
    return { enabled }
  }))
