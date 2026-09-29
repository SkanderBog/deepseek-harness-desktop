import type { IncompatibleVersion, PolicyBlockedVersion } from '../preinstall/types'
import type { DshPlugin, PluginErrorInfo } from '@/types'

export type BlockedRefusal
  = | { kind: 'incompatible', versions: IncompatibleVersion[] }
    | { kind: 'policy', versions: PolicyBlockedVersion[] }
    | { kind: 'update-hold', versions: PolicyBlockedVersion[], retryable: boolean }

export interface Plugin {
  id: string
  name: string
  version: string
  description: string
  repoUrl: string
  bundled: boolean
  disabled: boolean
  patchDisabled: boolean
  recommended: boolean
  fix: boolean
  internal: boolean
  hasSnapshot: boolean
  error: PluginErrorInfo | null
  latest: string | null
  updateAvailable: boolean
  incompatible: boolean
  latestIncompatible: boolean
}

export type PluginRef = string | { spec: string, version?: string }

export type PluginProcessType = 'install' | 'upgrade' | 'uninstall' | 'disable' | 'enable'

export type PluginProcessStatus = 'pending' | 'running' | 'unauthorized'

export type PluginProcessReason
  = | 'not-installed'
    | 'already-absent'
    | 'update-hold'
    | 'rejected'
    | 'cancelled'
    | 'retry-exhausted'

export interface PluginProcess {
  id: string
  groupId: string
  type: PluginProcessType
  status: PluginProcessStatus
  spec: string
  name: string
  version?: string
  refusal?: BlockedRefusal
  progressKey?: string
  approvalKey?: string
}

export interface PluginProcessResult {
  process: PluginProcess
  ok: boolean
  error?: string
  code?: string
  reason?: PluginProcessReason
}

export interface PluginManagerLog {
  at: number
  level: 'info' | 'error'
  message: string
  groupId?: string
  processId?: string
}

export type PluginSearchProblem = 'invalid-spec' | 'not-found' | 'network' | 'unsupported' | 'unknown'

export interface PluginSearchResult {
  spec: string
  name?: string
  version?: string
  compatible: boolean | null
  peers?: Record<string, string>
  problem?: PluginSearchProblem
}

export interface PluginsManagerRuntime {
  toast: boolean
  restartOnSettle: boolean
  clearConfigOverride?: boolean
}

export interface PluginGroup {
  id: string
  type: PluginProcessType
  status: 'pending' | 'active' | 'settled'
  processIds: string[]
  attempt: number
  pending: string[]
  results: PluginProcessResult[]
  resume?: () => void
  done: Promise<PluginProcessResult[]>
  resolveDone: (results: PluginProcessResult[]) => void
  options: PluginsManagerRuntime
}

export interface PluginsState {
  groups: PluginGroup[]
  processes: PluginProcess[]
  logs: PluginManagerLog[]
  activeGroupId: string | null
  cancelling: boolean
  presenterCount: number
  installedSource: DshPlugin[]
  installedLoaded: boolean
}

export type { DshPlugin }
