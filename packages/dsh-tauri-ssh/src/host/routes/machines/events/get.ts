import type { EventHandlerRequest } from 'dsh-tauri'
import type { SshMachineEventsQuery, SshMachineEventsResponse } from '../../index.types'
import { defineEventHandler, getQuery } from 'dsh-tauri'
import { events } from '../../../service/events'
import { guarded, machineIdOf, sinceSeqOf } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SshMachineEventsResponse>>(async event =>
  guarded(event, async () => {
    const query = getQuery<SshMachineEventsQuery>(event)
    const page = events.since(machineIdOf(query), sinceSeqOf(query))
    return { items: page.events, nextSeq: page.nextSeq }
  }))
