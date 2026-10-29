import type { ServiceResult } from '../types/index'
import type { SyncApplyInput } from './sync.types'
import * as api from '../apis/index'
import { errorMessageOf, syncApplyResultOf, syncPreviewOf } from '../apis/parsers'
import { store } from '../store/index'

export async function loadSyncPreview(): Promise<void> {
  store.sync.beginLoad()
  try {
    const preview = syncPreviewOf(await api.getSyncPreview())
    if (preview === null)
      throw new Error('malformed sync.preview payload')
    store.sync.commitPreview(preview)
  }
  catch (error) {
    store.sync.failLoad(errorMessageOf(error))
  }
}

export async function applySync(input: SyncApplyInput): Promise<ServiceResult> {
  if (store.sync.applying)
    return { ok: false, error: 'sync already running' }
  const target = [...store.machines.machines, ...store.machines.discovered].find(machine => machine.id === input.machineId)
  store.sync.beginApply(input.machineId, target ?? null)
  try {
    const result = syncApplyResultOf(await api.postSyncApply({
      machineId: input.machineId,
      plugins: input.plugins.map(item => ({ name: item.name, spec: item.spec })),
      skills: input.skills.map(item => ({ name: item.name, root: item.root })),
    }))
    if (result === null)
      throw new Error('malformed sync.apply payload')
    store.sync.mergeResults(result.items)
    return { ok: true }
  }
  catch (error) {
    const message = errorMessageOf(error)
    store.sync.fail(message)
    return { ok: false, error: message }
  }
  finally {
    store.sync.endApply()
  }
}
