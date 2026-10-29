export interface MigrationBackup {
  path: string
  stamp: string
  sessions: number
}

export interface DataDirEntry {
  name: string
  path: string
  files: number
  bytes: number
}

export interface MigrationPlan {
  source: string
  target: string
  totalFiles: number
  totalBytes: number
  links: number
  targetFreeBytes: number
  enoughSpace: boolean
  parentExists: boolean
  remembered: boolean
}

export interface MigrationOutcome {
  source: string
  target: string
  movedTo: string
  files: number
  bytes: number
  links: number
  sessions: number
}

export interface DataDirProgress {
  operation: 'migrate' | 'rollback'
  phase: 'scan' | 'copy' | 'verify' | 'finalize' | 'done'
  copiedFiles: number
  copiedBytes: number
  totalFiles: number
  totalBytes: number
}

export interface DataDirStatus {
  supported: boolean
  dataDir: string
  defaultDir: string
  envOverride: string
  rollbackAvailable: boolean
  backups: MigrationBackup[]
  debugBuild: boolean
}
