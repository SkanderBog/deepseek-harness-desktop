import type { RecoveryState } from './types'

export function createRecovery(attempts = 0): RecoveryState {
  return {
    required: false,
    info: null,
    attempts,
    busy: false,
  }
}
