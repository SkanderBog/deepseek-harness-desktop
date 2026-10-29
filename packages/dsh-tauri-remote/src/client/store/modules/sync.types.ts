import type { MachineRow, SyncItemResult, SyncPreview } from '../../types/index'

export type SyncTarget = Pick<MachineRow, 'id' | 'name' | 'host' | 'port' | 'user' | 'profileName'>

export type SyncStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface SyncState {
  status: SyncStatus
  error: string | null
  preview: SyncPreview | null
  applying: boolean
  machineId: string | null
  target: SyncTarget | null
  results: SyncItemResult[] | null
}
