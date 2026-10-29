import type { UnlistenFn } from '@tauri-apps/api/event'
import type { IncompatibleVersion, PolicyBlockedVersion, PreinstallLogPayload, PreinstallPlugin, PreinstallSelection } from './types'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import i18next from 'i18next'
import { defineStore } from 'valtio-define'
import { hooks } from '@/config/hooks'
import { harness } from '../harness'
import { harnessUpdater } from '../harness-updater'
import { parseBlockedRefusal } from '../plugins/utils'

const LOG_LIMIT = 200

export { parseBlockedRefusal }
export type { BlockedRefusal } from '../plugins/types'

export const preinstall = defineStore({
  state: () => ({
    plugins: [] as PreinstallPlugin[],
    loading: false,
    installing: false,
    cancelling: false,
    logs: [] as string[],
    error: '',
    incompatible: [] as IncompatibleVersion[],
    policyBlocked: [] as PolicyBlockedVersion[],
    loadError: '',
    isFirstTime: true,
  }),
  actions: {
    async load(): Promise<PreinstallPlugin[]> {
      if (this.loading)
        return this.plugins
      this.loading = true
      try {
        this.plugins = await invoke<PreinstallPlugin[]>('get_preinstall_plugins')

        this.loadError = ''
      }
      catch (err) {
        console.error('[Harness] failed to load preinstall plugins:', err)

        this.loadError = String(err)
      }
      finally {
        this.loading = false
      }
      return this.plugins
    },

    pushLog(line: string) {
      this.logs = [...this.logs, line].slice(-LOG_LIMIT)
    },

    async listenLog(): Promise<UnlistenFn> {
      return listen<PreinstallLogPayload>('preinstall-log', (e) => {
        this.pushLog(e.payload.line)
      })
    },

    async confirm(input: PreinstallSelection | string[]) {
      const installIds = Array.isArray(input) ? input : (input.installIds ?? [])
      const uninstallIds = Array.isArray(input) ? [] : (input.uninstallIds ?? [])
      if (this.installing || (installIds.length === 0 && uninstallIds.length === 0))
        return
      this.installing = true
      this.error = ''
      this.incompatible = []
      this.logs = []
      let unlisten: UnlistenFn | null = null
      try {
        unlisten = await this.listenLog()
        await invoke('install_preinstall_plugins', { installIds, uninstallIds })
        // issue #48: 后端装完已停服务，日志里先讲清接下来的自动重启。
        this.logs = [...this.logs, i18next.t('preinstall.restarting_hint')].slice(-LOG_LIMIT)
        await this.continueStartup()
      }
      catch (err) {
        console.error('[Harness] preinstall failed:', err)
        const error = String(err)
        const refusal = parseBlockedRefusal(error)
        if (refusal?.kind === 'incompatible') {
          this.incompatible = refusal.versions
        }
        else if (refusal?.kind === 'policy') {
          this.policyBlocked = refusal.versions
        }
        else {
          this.error = error.startsWith('NETWORK_ERROR:')
            ? i18next.t('preinstall.network_error')
            : error
        }
      }
      finally {
        unlisten?.()
        this.installing = false
        this.cancelling = false
      }
    },

    async allowIncompatible(versions: IncompatibleVersion[]): Promise<boolean> {
      if (this.installing || versions.length === 0)
        return false
      try {
        await invoke('allow_plugin_versions', { versions })
        this.incompatible = []
        return true
      }
      catch (err) {
        console.error('[Harness] granting plugin version exemption failed:', err)
        this.incompatible = []
        this.error = String(err)
        return false
      }
    },

    async allowPolicyVersions(versions: PolicyBlockedVersion[], expectedProfile?: string): Promise<boolean> {
      if (this.installing || versions.length === 0)
        return false
      try {
        await invoke('allow_plugin_policy_versions', { versions, ...(expectedProfile === undefined ? {} : { expectedProfile }) })
        this.policyBlocked = []
        return true
      }
      catch (err) {
        console.error('[Harness] granting release-age exemption failed:', err)
        this.policyBlocked = []
        this.error = String(err)
        return false
      }
    },

    async cancel() {
      if (!this.installing || this.cancelling)
        return

      this.cancelling = true

      let unlisten: (() => void) | undefined
      try {
        unlisten = await listen<unknown>('preinstall-cancelled', () => {
          this.installing = false
          this.cancelling = false
        })
        await invoke('cancel_plugin_processes')
      }
      catch (err) {
        console.error('[Harness] cancel preinstall failed:', err)
        this.cancelling = false
      }
      finally {
        unlisten?.()
      }
    },

    async skip() {
      if (this.installing)
        return
      await this.finish()
    },

    async finish() {
      try {
        await invoke('skip_preinstall_plugins')
        await this.continueStartup()
      }
      catch (err) {
        console.error('[Harness] finish preinstall failed:', err)
        this.error = String(err)
      }
    },

    async continueStartup() {
      await harness.launchAndWait()
      void harnessUpdater.checkForUpdate()
    },

    async open() {
      if (this.installing)
        return
      void hooks['config.dialog.hidden'].trigger()
      this.error = ''

      this.incompatible = []
      this.policyBlocked = []
      this.logs = []

      this.isFirstTime = false
      harness.status = 'preinstall'
      await this.load()
    },
  },
})
