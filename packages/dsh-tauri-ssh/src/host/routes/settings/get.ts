import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshSettingsResponse } from '../index.types'
import { defineEventHandler } from 'dsh-tauri'
import { machine } from '../../service/machine'
import { guarded } from '../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshSettingsResponse>>(async event =>
  guarded(event, async () => ({ enabled: machine.enabled() })))
