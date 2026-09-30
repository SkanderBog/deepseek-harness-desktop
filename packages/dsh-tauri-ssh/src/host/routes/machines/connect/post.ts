import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshConnectResponse, SshMachineIdBody } from '../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../../service/machine'
import { guarded, machineIdOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshConnectResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshMachineIdBody>(event)) ?? {}
    const link = await machine.connect(machineIdOf(body), new AbortController().signal)
    return { tunnelBaseUrl: link.tunnelBaseUrl }
  }))
