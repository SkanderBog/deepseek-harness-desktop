import type { Appearance } from '../../../../../../../packages/builtin-plugin-tauri/src/shared/appearance'

export interface AppSettingUpdate {
  proxyUrl?: string
  appearance?: Appearance
  port?: number
  zoomFactor?: number
  harnessMaxHeapMb?: number
  autoStart?: boolean
  autoUpdate?: boolean
  cliLinkEnabled?: boolean
  closeAction?: string
  backupRetentionCount?: number
  backupIncludeCredentials?: boolean
}

export type ZoomAction = 'increase' | 'decrease' | 'reset'
