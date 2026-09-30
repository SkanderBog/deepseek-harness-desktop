import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshActionResponse, SshMachineSaveBody } from '../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../service/machine'
import { guarded, machineIdOf, saveRowOf, secretsOf } from '../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshActionResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshMachineSaveBody>(event)) ?? {}
    await machine.save(machineIdOf(body), saveRowOf(body), secretsOf(body))
    return {}
  }))
