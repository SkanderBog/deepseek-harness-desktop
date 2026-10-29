export interface PluginRecoveryInfo {
  plugins: string[]
  reason: string
  detail: string
  rawError: string
}

export interface RecoveryState {
  required: boolean
  info: PluginRecoveryInfo | null
  attempts: number
  busy: boolean
}

export const MAX_RECOVERY_ATTEMPTS = 3
