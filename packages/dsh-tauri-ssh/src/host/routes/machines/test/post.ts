import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshMachineIdBody, SshTestResponse } from '../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../../service/machine'
import { guarded, machineIdOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshTestResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshMachineIdBody>(event)) ?? {}
    return await machine.test(machineIdOf(body), new AbortController().signal)
  }))
