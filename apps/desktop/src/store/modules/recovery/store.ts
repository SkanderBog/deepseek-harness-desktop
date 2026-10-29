import type { PluginRecoveryInfo } from './types'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { defineStore } from 'valtio-define'
import { harness } from '../harness'
import { MAX_RECOVERY_ATTEMPTS } from './types'
import { createRecovery } from './utils'

export const recovery = defineStore({
  state: () => ({
    recovery: createRecovery(),
    dismissedRecoveryIds: [] as string[],
  }),
  getters: {
    exhausted(): boolean {
      return this.recovery.attempts >= MAX_RECOVERY_ATTEMPTS
    },
  },
  actions: {
    async listen() {
      try {
        await listen<PluginRecoveryInfo>('plugin-recovery-required', (event) => {
          this.setRuntimeRecovery(event.payload)
        })
      }
      catch (err) {
        console.error('[Harness] failed to listen plugin-recovery-required:', err)
      }
    },

    setRuntimeRecovery(info: PluginRecoveryInfo) {
      if (info.plugins.length === 0)
        return
      if (info.plugins.some(id => this.dismissedRecoveryIds.includes(id)))
        return
      this.recovery = { required: true, info, attempts: this.recovery.attempts + 1, busy: false }
    },

    async reviewStartupRecovery(logs: string[], shouldCommit?: () => boolean) {
      if (this.recovery.required || logs.length === 0)
        return
      try {
        const info = await invoke<PluginRecoveryInfo>('detect_plugin_recovery', { logs })
        if (shouldCommit && !shouldCommit())
          return
        if (info.plugins.length > 0) {
          this.recovery = {
            required: true,
            info,
            attempts: this.recovery.attempts + 1,
            busy: false,
          }
        }
      }
      catch (err) {
        console.error('[Harness] detect_plugin_recovery failed:', err)
      }
    },

    async recoverAndRedetect(ids: readonly string[]) {
      if (this.recovery.busy || ids.length === 0)
        return
      this.recovery = { ...this.recovery, busy: true }
      try {
        for (const id of ids) {
          await invoke('recover_plugin', { id })
        }

        this.dismissedRecoveryIds = this.dismissedRecoveryIds.filter(x => !ids.includes(x))
        this.clear()
        await harness.restart()
      }
      catch (err) {
        console.error('[Harness] recover_plugin failed:', err)
        this.recovery = { ...this.recovery, busy: false, attempts: this.recovery.attempts + 1 }
      }
    },

    async restoreAndRedetect(ids: readonly string[]) {
      if (this.recovery.busy || ids.length === 0)
        return
      this.recovery = { ...this.recovery, busy: true }
      try {
        for (const id of ids) {
          try {
            await invoke('restore_plugin', { id })
          }
          catch (err) {
            console.error(`[Harness] restore_plugin failed for ${id}:`, err)
          }
        }
        this.clear()
        await harness.restart()
      }
      catch (err) {
        console.error('[Harness] restoreAndRedetect failed:', err)
        this.recovery = { ...this.recovery, busy: false, attempts: this.recovery.attempts + 1 }
      }
    },

    dismissRecovery() {
      if (this.recovery.info) {
        this.dismissedRecoveryIds = [
          ...new Set([...this.dismissedRecoveryIds, ...this.recovery.info.plugins]),
        ]
      }
      this.recovery = createRecovery()
    },

    reset() {
      this.recovery = createRecovery()
      this.dismissedRecoveryIds = []
    },

    clear() {
      this.recovery = createRecovery(this.recovery.attempts)
    },

    hide() {
      this.recovery = { ...this.recovery, required: false, busy: false }
    },
  },
})
