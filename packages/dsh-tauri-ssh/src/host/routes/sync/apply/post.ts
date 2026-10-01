import type { EventHandlerRequest } from 'dsh-tauri'
import type { SyncApplyBody, SyncApplyResponse } from '../../index.types'
import { defineEventHandler, readBody } from 'dsh-tauri'
import { sync } from '../../../service/sync'
import { guarded, machineIdOf, pluginRefsOf, skillRefsOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SyncApplyResponse>>(async event =>
  guarded(event, async () => {
    const body = (await readBody<SyncApplyBody>(event)) ?? {}
    return await sync.apply(machineIdOf(body), pluginRefsOf(body), skillRefsOf(body))
  }))
