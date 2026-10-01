import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshActionResponse, SshMachineIdBody } from '../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../../service/machine'
import { guarded, machineIdOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshActionResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshMachineIdBody>(event)) ?? {}
    await machine.disconnect(machineIdOf(body))
    return {}
  }))
