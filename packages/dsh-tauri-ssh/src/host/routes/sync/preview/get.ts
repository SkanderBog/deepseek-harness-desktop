import type { EventHandlerRequest } from 'dsh-tauri'
import type { SyncPreviewResponse } from '../../index.types'
import { defineEventHandler } from 'dsh-tauri'
import { sync } from '../../../service/sync'
import { guarded } from '../../index.utils'

export default defineEventHandler<EventHandlerRequest, Promise<SyncPreviewResponse>>(async event =>
  guarded(event, async () => sync.preview()))
