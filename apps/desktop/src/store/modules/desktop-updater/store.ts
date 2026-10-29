import type {
  DesktopAboutInfo,
  DesktopDownloadProgress,
  DesktopUpdateInfo,
} from './types'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import i18next from 'i18next'
import { defineStore } from 'valtio-define'
import { toast } from '@/utils/toast'
import { setting } from '../setting'

let checkTask: Promise<DesktopUpdateInfo | null> | null = null
let downloadTask: Promise<void> | null = null
let downloadAndOpenTask: Promise<boolean> | null = null

export const desktopUpdater = defineStore({
  state: () => ({
    updateInfo: null as DesktopUpdateInfo | null,
    checking: false,
    downloading: false,
    downloadProgress: 0,
    about: null as DesktopAboutInfo | null,
  }),
  actions: {
    check(): Promise<DesktopUpdateInfo | null> {
      if (checkTask)
        return checkTask
      this.checking = true
      const task = (async () => {
        try {
          const info = await invoke<DesktopUpdateInfo | null>('check_desktop_update')
          if (info) {
            this.updateInfo = info
            // issue #1132: 自动更新关闭后手动检查只展示对话框，下载留给「立即更新」。
            if (setting.auto_update)
              void this.download()
          }
          else {
            this.updateInfo = null
          }
          return info
        }
        finally {
          checkTask = null
          this.checking = false
        }
      })()
      checkTask = task
      return task
    },

    download(): Promise<void> {
      if (downloadTask)
        return downloadTask
      const info = this.updateInfo
      if (!info || info.downloaded)
        return Promise.resolve()

      this.downloading = true
      this.downloadProgress = 0
      downloadTask = invoke<DesktopUpdateInfo>('download_desktop_update')
        .then((updated) => {
          this.updateInfo = updated
        })
        .catch((err) => {
          console.error('[DesktopUpdater] download failed:', err)
        })
        .finally(() => {
          downloadTask = null
          this.downloading = false
          this.downloadProgress = 0
        })
      return downloadTask
    },

    async loadAbout(): Promise<DesktopAboutInfo | null> {
      if (!this.about) {
        try {
          this.about = await invoke<DesktopAboutInfo>('get_desktop_about')
        }
        catch (err) {
          console.warn('[DesktopUpdater] failed to load about info:', err)
        }
      }
      return this.about
    },

    downloadAndOpen(): Promise<boolean> {
      if (downloadAndOpenTask)
        return downloadAndOpenTask
      const task = (async () => {
        const info = this.updateInfo
        if (!info)
          return false

        let path = info.path
        if (!info.downloaded) {
          await this.download()
          const updated = this.updateInfo
          if (!updated?.downloaded) {
            toast(i18next.t('update.desktop_download_failed'), {
              variant: 'danger',
              placement: 'bottom end',
            })
            return false
          }
          path = updated.path
        }

        try {
          await invoke('open_desktop_installer', { path })
          toast(i18next.t('update.desktop_opened'), {
            variant: 'default',
            placement: 'bottom end',
          })
          return true
        }
        catch (err) {
          console.error('[DesktopUpdater] failed to open installer:', err)
          toast(i18next.t('update.desktop_open_failed'), {
            variant: 'danger',
            placement: 'bottom end',
          })
          return false
        }
      })().finally(() => {
        downloadAndOpenTask = null
      })
      downloadAndOpenTask = task
      return task
    },
  },
})

listen<DesktopDownloadProgress>('desktop-update-progress', (e) => {
  desktopUpdater.downloadProgress = e.payload.percentage

  if (e.payload.message) {
    toast(e.payload.message, {
      variant: 'default',
      placement: 'bottom end',
    })
  }
}).catch((err) => {
  console.error('[DesktopUpdater] failed to listen desktop-update-progress:', err)
})
