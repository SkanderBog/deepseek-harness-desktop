import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshMachinesResponse } from '../index.types'
import { defineEventHandler } from 'dsh-tauri'
import { machine } from '../../service/machine'
import { guarded, listItemOf } from '../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshMachinesResponse>>(async event =>
  guarded(event, async () => {
    if (!machine.enabled())
      return { enabled: false, items: [], discovered: [] }
    const items = machine.profileViews().map(view => listItemOf(view, machine.status(view.id)))
    const discovered = (await machine.discoveredViews()).map(view => listItemOf(view, machine.status(view.id)))
    return { enabled: true, items, discovered }
  }))
