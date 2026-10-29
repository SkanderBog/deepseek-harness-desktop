export interface PluginErrorInfo {
  message: string
  action: string
  at: string
}

export interface DshPlugin {
  id: string
  name: string
  version: string
  description: string
  repo_url: string
  bundled: boolean
  disabled: boolean
  recommended: boolean
  fix: boolean
  internal: boolean
  updateAvailable: boolean
  latestVersion?: string
  hasSnapshot: boolean
  error?: PluginErrorInfo | null
}

export interface QuarantinedPatchLayer {
  original: string
  backup: string
  error: string
}

export interface PatchQuarantineFailure {
  path: string
  error: string
}

export interface PatchQuarantineReport {
  quarantined: QuarantinedPatchLayer[]
  failures: PatchQuarantineFailure[]
}

export interface StrippedPatchLayer {
  original: string
  backup: string
  removed: number
}

export interface PatchEntryStripReport {
  layers: StrippedPatchLayer[]
}

export type MigrationVerdict = 'compatible' | 'upgrade' | 'downgrade' | 'unknown'

export interface MigrationEntry {
  id: string
  version: string
  spec: string
  verdict: MigrationVerdict
  targetVersion?: string
}

export type MigrationDataKind = 'patch' | 'policy' | 'credentials'

export interface MigrationDataItem {
  kind: MigrationDataKind
  count?: number
  covered: boolean
}

export interface MigrationAnalysis {
  plugins: MigrationEntry[]
  data: MigrationDataItem[]
}

export interface MigrationFailure {
  kind: MigrationDataKind
  message: string
}

export interface MigrateReport {
  applied: MigrationDataKind[]
  skipped: MigrationDataKind[]
  failures: MigrationFailure[]
}
