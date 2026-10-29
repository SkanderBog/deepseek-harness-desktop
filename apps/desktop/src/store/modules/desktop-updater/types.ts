export interface DesktopUpdateInfo {
  version: string
  currentVersion: string
  tag: string
  published_at: string
  url: string
  asset_name: string
  path: string
  downloaded: boolean
}

export interface DesktopDownloadProgress {
  percentage: number
  downloaded: number
  total: number
  message?: string
}

export interface DesktopAboutInfo {
  version: string
  publishedAt: string
  copyright: string
  repo: string
  poweredBy: string
}
