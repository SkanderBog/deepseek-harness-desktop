import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshInstallResponse, SshMachineIdBody } from '../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { machine } from '../../../service/machine'
import { guarded, machineIdOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshInstallResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SshMachineIdBody>(event)) ?? {}
    return await machine.install(machineIdOf(body), new AbortController().signal)
  }))
