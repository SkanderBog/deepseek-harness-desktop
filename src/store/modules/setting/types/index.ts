export interface AppSettingUpdate {
  port?: number
  harnessMaxHeapMb?: number
  autoStart?: boolean
  cliLinkEnabled?: boolean
  closeAction?: string
  backupRetentionCount?: number
  backupIncludeCredentials?: boolean
}

export type ZoomAction = 'increase' | 'decrease' | 'reset'
