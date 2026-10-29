import type { UnlistenFn } from '@tauri-apps/api/event'
import type { DshUpdateInfo } from './types'
import { invoke } from '@tauri-apps/api/core'
import i18next, { t } from 'i18next'
import { defineStore } from 'valtio-define'
import { toast } from '@/utils/toast'
import { harness } from '../harness'

export const harnessUpdater = defineStore({
  state: () => ({
    updateInfo: null as DshUpdateInfo | null,
    updating: false,
  }),
  actions: {
    async checkForUpdate() {
      try {
        const info = await invoke<DshUpdateInfo | null>('check_dsh_update')

        this.updateInfo = info
      }
      catch (err) {
        console.warn('[Harness] update check skipped:', err)
      }
    },

    async handleUpdate(): Promise<boolean> {
      if (this.updating)
        return false
      this.updating = true
      let unlistenInstall: UnlistenFn | null = null
      let installed = false
      try {
        unlistenInstall = await harness.listenInstallProgress()
        harness.prepareInstall(i18next.t('status.updating'))

        const changed = await invoke<boolean>('install_dependencies')
        installed = changed
        if (!changed) {
          harness.status = 'ready'

          this.updateInfo = null
          try {
            await this.checkForUpdate()
          }
          catch {
          }
          if (this.updateInfo) {
            toast(i18next.t('update.verify_failed'), { variant: 'danger' })
          }
          return false
        }
        await harness.launchAndWait()
        this.updateInfo = null
        return true
      }
      catch (err) {
        console.error('[Harness] update failed:', err)
        harness.fail(String(err))
        return installed
      }
      finally {
        unlistenInstall?.()
        this.updating = false
      }
    },

    dismissUpdate() {
      this.updateInfo = null
    },

    showToast(onUpdate: () => void) {
      if (!this.updateInfo)
        return
      toast(t('update.available', { tag: this.updateInfo.tag }), {
        actionProps: {
          children: t('update.now'),
          onPress: () => {
            toast.clear()
            onUpdate()
          },
          variant: 'tertiary',
        },
        placement: 'bottom end',
        description: this.updateInfo.commit.slice(0, 7),
        variant: 'default',
      })
    },
  },
})
