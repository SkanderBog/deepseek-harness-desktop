export type CoreSource = 'local' | 'app'

export interface HarnessCore {
  id: string
  source: CoreSource
  version: string
  tag: string
  path: string
  dir: string
  present: boolean
  active: boolean
  /** 能否卸载：磁盘上存在独立槽位目录才有落点；就地安装的激活副本没有槽位（issue #790） */
  removable: boolean
  preview: boolean
  aboveRecommended: boolean
  orphaned: boolean
  bundled: boolean
  recommendedVersion: string | null
  error?: string | null
}
