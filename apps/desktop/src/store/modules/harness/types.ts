import type { StartupPhase } from './readiness'

export type SetupStatus = 'checking' | 'installing' | 'starting' | 'preinstall' | 'ready' | 'error'

export type SidebarBusyAction = 'repair' | 'restart' | 'shutdown' | 'start' | 'openBrowser' | null

export type PatchRepairAction = 'safe-mode' | 'quarantine' | 'strip'

export interface HarnessProcessExitedPayload {
  pid: number
  exitCode: number | null
}

export const INTERNAL_PLUGIN_PHASE_DETAILS = [
  'waiting',
  'checking',
  'installing',
  'heartbeat',
  'done',
  'timeout',
  'cancelled',
] as const

export type InternalPluginPhaseDetail = typeof INTERNAL_PLUGIN_PHASE_DETAILS[number]

export interface InternalPluginsPhasePayload {
  phase: 'loading' | 'progress' | 'done'
  detail: InternalPluginPhaseDetail
  completed: number
  total: number
}

export interface InstallerState {
  title: string
  detail: string
  percentage: number
  logs: string[]
}

export interface InstallProgress {
  title: string
  detail: string
  log: string
  type: string
  percentage: number
  progress: number
}

export interface StartupError extends Error {
  logs?: string[]
  logLines?: string[]
  pluginConflictHint?: string
  inotifyLimitHint?: string
  heapOomHint?: string
  patchLayerHint?: string
  phase?: StartupPhase
  lastReason?: string
}
