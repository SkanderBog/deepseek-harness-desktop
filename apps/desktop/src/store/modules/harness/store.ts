import type { UnlistenFn } from '@tauri-apps/api/event'
import type { PolicyBlockedVersion } from '../preinstall'
import type { ReadinessProbeResult, StartupPhase } from './readiness'
import type {
  HarnessProcessExitedPayload,
  InstallerState,
  InstallProgress,
  InternalPluginsPhasePayload,
  PatchRepairAction,
  SetupStatus,
  SidebarBusyAction,
  StartupError,
} from './types'
import type { PatchEntryStripReport, PatchQuarantineReport } from '@/types/plugin'
import type { Profile } from '@/types/profile'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import i18next from 'i18next'
import semver from 'semver'
import { defineStore } from 'valtio-define'
import { queryClient } from '@/config/client'
import { hooks } from '@/config/hooks'
import { queryKeys } from '@/config/query-keys'
import { toast } from '@/utils/toast'
import { harnessUpdater } from '../harness-updater'
import { parseBlockedRefusal } from '../plugins'
import { preinstall } from '../preinstall'
import { recovery } from '../recovery'
import { setting } from '../setting'
import {
  IFRAME_FRAME_GRACE_TIMEOUT,
  IFRAME_LOAD_TIMEOUT,
  IFRAME_RECOVERY_ABSOLUTE_TIMEOUT,
  IFRAME_RELOAD_MAX_ATTEMPTS,
  PLUGIN_ABSOLUTE_TIMEOUT,
  PLUGIN_ACTIVITY_CHECK_INTERVAL,
  PLUGIN_INACTIVITY_TIMEOUT,
  STARTUP_ABSOLUTE_TIMEOUT,
} from './constants'
import { BoundedReloadGate, SingleFlight, waitForActivityTask } from './readiness'
import { runtimeExitMessageKey, shouldAcceptRuntimeExit } from './runtime'
import {
  attachStartupDiagnostics,
  checkHealthViaProxy,
  consumePortConflictRestart,
  generateTimestampedUrl,
  internalPluginReason,
  notifyPatchEntryStrip,
  notifyPatchQuarantine,
  pollHarnessReadiness,
  resetPortConflictRestarts,
  startupError,
} from './utils'

const STARTUP_STATUS_KEYS: Record<StartupPhase, string> = {
  'plugin-install': 'status.loading_internal',
  'process-boot': 'status.loading_process',
  'client-modules': 'status.loading_client_modules',
}

const PATCH_REPAIR_LOG: Record<PatchRepairAction, string> = {
  'safe-mode': '[Harness] enter safe mode failed:',
  'quarantine': '[Harness] quarantine broken patch layers failed:',
  'strip': '[Harness] strip unresolved patch entries failed:',
}

const initialInstaller: InstallerState = {
  title: '',
  detail: '',
  percentage: 0,
  logs: [],
}

let bootToken = 0
let readinessCommitToken: number | null = null
let bootStarted = false
let pluginActivitySequence = 0
let pluginActivityReason = ''
const restartFlight = new SingleFlight<void>()
const iframeReloadGate = new BoundedReloadGate(IFRAME_RELOAD_MAX_ATTEMPTS)
let iframeRefreshTimer: ReturnType<typeof setTimeout> | undefined
let policyApprovalKey: string | undefined

export const harness = defineStore({
  state: () => ({
    status: 'ready' as SetupStatus,
    installer: initialInstaller,
    errorMsg: '',
    errorLogs: [] as string[],
    pluginConflictHint: '',
    inotifyLimitHint: '',
    patchLayerHint: '',
    heapOomHint: '',
    serviceUrl: 'http://127.0.0.1:3080',
    iframeSrc: '',
    frontendGeneration: 0,
    frontendBootHtml: null as string | null,
    iframeLoaded: false,
    iframeError: false,
    // issue #705: load 事件不足以证明页面已起来（浏览器错误页同样触发 load），只认帧内自报。
    iframeAliveKey: null as number | null,
    iframeKey: 0,
    serviceHealthy: false,
    serviceRunning: false,
    startupPhase: 'plugin-install' as StartupPhase,
    startupReason: '',
    busyAction: null as SidebarBusyAction,
  }),
  getters: {
    startupStatusKey(): string {
      return STARTUP_STATUS_KEYS[this.startupPhase]
    },
    showIframeError(): boolean {
      return this.serviceHealthy && this.iframeError
    },
    iframeAlive(): boolean {
      return this.iframeAliveKey === this.iframeKey
    },
    iframeErrorHint(): string {
      return this.iframeAlive ? '' : i18next.t('ui.iframe_unreachable_hint')
    },
  },
  actions: {
    resetFrontendPreload() {
      this.frontendGeneration++
      this.frontendBootHtml = null
    },

    startup() {
      if (bootStarted)
        return
      bootStarted = true
      void this.initialize()
    },

    async initialize() {
      await Promise.all([
        recovery.listen(),
        this.listenInternalPhase(),
        this.listenProcessExit(),
      ])
      await this.boot()
    },

    async listenProcessExit() {
      try {
        await listen<HarnessProcessExitedPayload>('harness-process-exited', (event) => {
          void this.handleProcessExit(event.payload)
        })
      }
      catch (err) {
        console.error('[Harness] failed to listen harness-process-exited:', err)
      }
    },

    async handleProcessExit(payload: HarnessProcessExitedPayload) {
      const observedToken = bootToken
      if (!shouldAcceptRuntimeExit({
        serviceHealthy: this.serviceHealthy,
        serviceRunning: this.serviceRunning,
        readinessCommitPending: readinessCommitToken === observedToken,
        busyAction: this.busyAction,
        observedToken,
        currentToken: bootToken,
        notOwned: true,
      })) {
        return
      }

      const probe = await checkHealthViaProxy()
      if (!shouldAcceptRuntimeExit({
        serviceHealthy: this.serviceHealthy,
        serviceRunning: this.serviceRunning,
        readinessCommitPending: readinessCommitToken === observedToken,
        busyAction: this.busyAction,
        observedToken,
        currentToken: bootToken,
        notOwned: probe.notOwned,
      })) {
        return
      }

      const exitToken = ++bootToken
      this.resetFrontendPreload()
      const message = i18next.t(runtimeExitMessageKey(payload.exitCode), {
        code: payload.exitCode,
      })
      let exitDetail = '(exit code unavailable)'
      if (payload.exitCode != null)
        exitDetail = `(exit code ${payload.exitCode})`
      console.warn(
        `[Harness] owned process ${payload.pid} exited unexpectedly`,
        exitDetail,
      )
      iframeReloadGate.reset()
      if (iframeRefreshTimer !== undefined) {
        clearTimeout(iframeRefreshTimer)
        iframeRefreshTimer = undefined
      }
      this.serviceHealthy = false
      this.iframeLoaded = false
      this.iframeError = false
      this.iframeAliveKey = null

      const error = await attachStartupDiagnostics(new Error(message), true)
      if (exitToken !== bootToken)
        return

      if (consumePortConflictRestart(error.logLines ?? [])) {
        console.warn('[Harness] port in use; restarting to pick another port')
        await this.boot()
        return
      }

      this.fail(message)
      await recovery.reviewStartupRecovery(
        error.logLines ?? error.logs ?? [],
        () => exitToken === bootToken,
      )
      if (exitToken !== bootToken)
        return
      this.fail(
        error.message,
        error.logs,
        error.pluginConflictHint,
        error.inotifyLimitHint,
        undefined,
        error.patchLayerHint,
        error.heapOomHint,
      )
    },

    async listenInternalPhase() {
      try {
        await listen<InternalPluginsPhasePayload>('internal-plugins-phase', (event) => {
          const payload = event.payload
          pluginActivitySequence++
          pluginActivityReason = internalPluginReason(
            payload,
            pluginActivityReason,
            (key, options) => i18next.t(key, options),
          )
          if (payload.phase !== 'done') {
            this.startupPhase = 'plugin-install'
            this.startupReason = pluginActivityReason
          }
        })
      }
      catch (err) {
        console.error('[Harness] failed to listen internal-plugins-phase:', err)
      }
    },

    refreshIframe() {
      this.iframeLoaded = false
      this.iframeError = false
      this.iframeAliveKey = null
      if (iframeRefreshTimer !== undefined) {
        clearTimeout(iframeRefreshTimer)
      }
      iframeRefreshTimer = setTimeout(() => {
        iframeRefreshTimer = undefined
        this.iframeKey++
      }, 800)
    },

    markIframeLoaded() {
      this.iframeLoaded = true
      this.iframeError = false
    },

    markIframeError() {
      this.iframeError = true
      this.iframeLoaded = false
    },

    markIframeAlive() {
      this.iframeAliveKey = this.iframeKey
    },

    markIframeLeaving() {
      this.iframeAliveKey = null
      this.iframeLoaded = false
    },

    markIframeBootReady() {
      iframeReloadGate.markReady()
      if (iframeRefreshTimer !== undefined) {
        clearTimeout(iframeRefreshTimer)
        iframeRefreshTimer = undefined
      }
    },

    async recoverIframeBoot() {
      const generation = this.iframeKey
      const decision = iframeReloadGate.request(generation)
      if (decision === 'duplicate' || decision === 'ready')
        return
      if (decision === 'exhausted') {
        const reason = i18next.t('errors.client_boot_stalled')
        const error = startupError('client-modules', reason, 'absolute')
        this.fail(error.message, undefined, undefined, undefined, this.serviceRunning)
        return
      }

      const result = await pollHarnessReadiness(
        IFRAME_RECOVERY_ABSOLUTE_TIMEOUT,
        () => generation === this.iframeKey && this.serviceRunning,
      )
      if (generation !== this.iframeKey || !this.serviceRunning)
        return
      if (result.healthy) {
        this.refreshIframe()
        return
      }

      const phase = result.phase ?? 'client-modules'
      const reason = result.reason ?? i18next.t('errors.no_readiness_reason')
      const kind = result.notOwned ? 'exited' : result.timeout ?? 'failed'
      const error = startupError(phase, reason, kind)
      this.fail(error.message, undefined, undefined, undefined, !result.notOwned)
    },

    async handleIframeBootFailure(detail?: string) {
      const message = detail && detail.includes('Failed to load plugins')
        ? detail
        : i18next.t('errors.client_boot_failed')
      const error = await attachStartupDiagnostics(new Error(message))

      const lines = [
        ...(detail ? detail.split(/\n/) : []),
        ...(error.logLines ?? error.logs ?? []),
      ]
      await recovery.reviewStartupRecovery(lines)
      this.fail(
        error.message,
        error.logs,
        error.pluginConflictHint,
        error.inotifyLimitHint,
        this.serviceRunning,
        error.patchLayerHint,
      )
    },

    async listenInstallProgress(): Promise<UnlistenFn> {
      return listen<InstallProgress>('install-progress', (e) => {
        const payload = e.payload
        if (payload.percentage < this.installer.percentage) {
          return
        }
        const logs = payload.log
          ? [...this.installer.logs, payload.log].slice(-5)
          : this.installer.logs
        this.installer = {
          title: payload.title || this.installer.title,
          detail: payload.detail || this.installer.detail,
          percentage: payload.percentage,
          logs,
        }
      })
    },

    async completeReadiness(token: number, probe?: ReadinessProbeResult): Promise<boolean> {
      if (token !== bootToken)
        return false

      let ownershipFailure: string | null = null
      try {
        await invoke('harness_ownership')
      }
      catch (err) {
        ownershipFailure = String(err)
      }
      if (token !== bootToken)
        return false
      if (ownershipFailure?.includes('HARNESS_NOT_OWNED')) {
        this.serviceRunning = false
        throw startupError('process-boot', ownershipFailure, 'exited')
      }

      const readyInfo = await invoke<{ service_url: string }>('get_runtime_info')
      if (token !== bootToken)
        return false

      this.serviceUrl = readyInfo.service_url
      this.iframeSrc = generateTimestampedUrl(readyInfo.service_url)
      this.frontendBootHtml = probe?.bootOrigin === readyInfo.service_url ? probe.bootHtml ?? null : null
      resetPortConflictRestarts()
      this.serviceHealthy = true
      this.serviceRunning = true
      this.status = 'ready'
      this.errorMsg = ''
      this.errorLogs = []
      this.pluginConflictHint = ''
      this.inotifyLimitHint = ''
      this.patchLayerHint = ''
      this.heapOomHint = ''
      preinstall.error = ''

      recovery.reset()
      iframeReloadGate.reset()
      if (iframeRefreshTimer !== undefined) {
        clearTimeout(iframeRefreshTimer)
        iframeRefreshTimer = undefined
      }

      void queryClient.invalidateQueries({ queryKey: queryKeys.info })
      void queryClient.invalidateQueries({ queryKey: queryKeys.cliStatus })

      void queryClient.invalidateQueries({ queryKey: queryKeys.plugins })
      void queryClient.invalidateQueries({ queryKey: queryKeys.profiles })
      void queryClient.invalidateQueries({ queryKey: queryKeys.cores })
      return true
    },

    async launchAndWait(token?: number) {
      const launchToken = token ?? ++bootToken
      if (launchToken !== bootToken)
        return
      if (token === undefined)
        this.resetFrontendPreload()
      this.frontendBootHtml = null
      this.status = 'ready'
      this.installer = initialInstaller
      this.errorMsg = ''
      this.errorLogs = []
      this.pluginConflictHint = ''
      this.inotifyLimitHint = ''
      this.patchLayerHint = ''
      this.heapOomHint = ''
      recovery.reset()
      this.serviceHealthy = false
      this.iframeLoaded = false
      this.iframeError = false
      this.iframeAliveKey = null
      this.startupPhase = 'process-boot'
      this.startupReason = i18next.t('status.loading_process')
      try {
        await invoke('launch_harness')
        if (launchToken !== bootToken)
          return
        this.serviceRunning = true
        this.startupPhase = 'process-boot'
        this.startupReason = i18next.t('status.loading_process')

        const runtimeInfo = await invoke<{ service_url: string }>('get_runtime_info')
        if (launchToken !== bootToken)
          return
        this.serviceUrl = runtimeInfo.service_url
        this.iframeSrc = generateTimestampedUrl(runtimeInfo.service_url)

        const result = await pollHarnessReadiness(
          STARTUP_ABSOLUTE_TIMEOUT,
          () => launchToken === bootToken,
          (probe) => {
            if (probe.phase) {
              this.startupPhase = probe.phase
            }
            this.startupReason = probe.reason ?? ''
          },
        )
        if (launchToken !== bootToken) {
          return
        }
        if (!result.healthy) {
          const phase = result.phase ?? this.startupPhase
          const reason = result.reason ?? (this.startupReason || i18next.t('errors.no_readiness_reason'))
          if (result.notOwned) {
            this.serviceRunning = false
            throw startupError(phase, reason, 'exited')
          }
          if (result.timeout) {
            throw startupError(phase, reason, result.timeout)
          }
          throw startupError(phase, reason, 'failed')
        }

        const readinessToken = launchToken
        readinessCommitToken = readinessToken
        try {
          await this.completeReadiness(readinessToken, result)
        }
        finally {
          if (readinessCommitToken === readinessToken)
            readinessCommitToken = null
        }
      }
      catch (err) {
        const error = err instanceof Error ? err : new Error(String(err))
        throw await attachStartupDiagnostics(error, error.message.includes('HARNESS_NOT_OWNED') || error.message.includes('Harness exited early'))
      }
    },

    async boot() {
      const token = ++bootToken
      this.resetFrontendPreload()
      if (policyApprovalKey !== undefined) {
        toast.close(policyApprovalKey)
        policyApprovalKey = undefined
      }
      iframeReloadGate.reset()
      if (iframeRefreshTimer !== undefined) {
        clearTimeout(iframeRefreshTimer)
        iframeRefreshTimer = undefined
      }

      this.serviceHealthy = false
      this.iframeLoaded = false
      this.iframeError = false
      this.iframeAliveKey = null

      this.errorMsg = ''
      this.errorLogs = []
      this.pluginConflictHint = ''
      this.inotifyLimitHint = ''
      this.patchLayerHint = ''
      this.heapOomHint = ''
      recovery.clear()
      this.status = 'ready'
      let unlistenInstall: UnlistenFn | null = null
      let policyApproval: { versions: PolicyBlockedVersion[], profile: string } | undefined

      try {
        try {
          unlistenInstall = await this.listenInstallProgress()
        }
        catch (err) {
          console.error('[Harness] failed to listen install-progress:', err)
        }
        const runtimeInfo = await invoke<{ service_url: string }>('get_runtime_info')
        this.serviceUrl = runtimeInfo.service_url
        this.iframeSrc = generateTimestampedUrl(runtimeInfo.service_url)

        await setting.refresh()

        const config = { ...setting.$state }

        const ready = await invoke<boolean>('runtime_ready')
        if (!ready || !config.installed) {
          if (!ready) {
            this.status = 'installing'
            this.installer = { ...initialInstaller, title: i18next.t('status.installing') }
          }
          await invoke('install_dependencies')
        }

        this.startupPhase = 'plugin-install'
        this.startupReason = i18next.t('status.loading_internal')
        pluginActivitySequence++
        pluginActivityReason = i18next.t('status.internal_waiting')
        const installProfile = (await invoke<Profile[]>('get_profiles')).find(item => item.active)?.id
        try {
          const result = await waitForActivityTask({
            task: invoke('ensure_internal_plugins'),
            getActivity: () => ({
              sequence: pluginActivitySequence,
              reason: pluginActivityReason,
            }),
            inactivityTimeoutMs: PLUGIN_INACTIVITY_TIMEOUT,
            absoluteTimeoutMs: PLUGIN_ABSOLUTE_TIMEOUT,
            intervalMs: PLUGIN_ACTIVITY_CHECK_INTERVAL,
            shouldContinue: () => token === bootToken,
          })
          if (result.cancelled) {
            return
          }
          if (result.timeout) {
            try {
              await invoke('cancel_internal_plugins')
            }
            catch (cancelError) {
              console.error('[Harness] failed to cancel timed-out internal plugin install:', cancelError)
              throw startupError('plugin-install', String(cancelError), 'failed')
            }
            throw startupError('plugin-install', result.reason, result.timeout)
          }
        }
        catch (err) {
          if (err instanceof Error && (err as StartupError).phase) {
            throw err
          }
          if (token !== bootToken)
            return
          const message = err instanceof Error ? err.message : String(err)
          const policyMessage = message.replace(/^INTERNAL_PLUGIN_INSTALL_FAILED: /, '')
          const refusal = policyMessage.startsWith('PLUGIN_POLICY_BLOCKED:') && !policyMessage.includes('\n')
            ? parseBlockedRefusal(policyMessage)
            : null
          if (refusal?.kind !== 'policy' || !Array.isArray(refusal.versions)
            || !refusal.versions.every(item => item && typeof item.name === 'string' && item.name.trim() !== ''
              && typeof item.version === 'string' && semver.valid(item.version))) {
            throw startupError('plugin-install', message, 'failed')
          }
          const profiles = await invoke<Profile[]>('get_profiles')
          const profile = profiles.find(item => item.active)?.id
          if (!installProfile || profile !== installProfile)
            throw startupError('plugin-install', message, 'failed')
          policyApproval = { versions: refusal.versions, profile }
        }

        if (await invoke<boolean>('get_preinstall_pending')) {
          this.status = 'preinstall'
          preinstall.isFirstTime = true
          await preinstall.load()
          return
        }

        await this.launchAndWait(token)

        if (token !== bootToken)
          return

        if (config.installed) {
          void harnessUpdater.checkForUpdate()
        }
      }
      catch (err) {
        if (token !== bootToken)
          return
        console.error('[Harness] startup failed:', err)
        const error = await attachStartupDiagnostics(err)

        await recovery.reviewStartupRecovery(error.logLines ?? error.logs ?? [])
        this.fail(
          error.message,
          error.logs,
          error.pluginConflictHint,
          error.inotifyLimitHint,
          this.serviceRunning,
          error.patchLayerHint,
          error.heapOomHint,
        )
      }
      finally {
        unlistenInstall?.()
        if (policyApproval && token === bootToken) {
          const { versions, profile } = policyApproval
          let granting = false
          const key = toast(i18next.t('plugins.blocked_policy_title', { name: versions.map(item => item.name).join('、') }), {
            variant: 'warning',
            timeout: 0,
            sticky: true,
            description: i18next.t('plugins.blocked_policy_desc', { blocked: versions.map(item => `${item.name}@${item.version}`).join('、') }),
            actionProps: {
              children: i18next.t('buttons.authorize'),
              onPress: async () => {
                if (granting || token !== bootToken || this.busyAction || preinstall.installing)
                  return
                granting = true
                try {
                  const profiles = await invoke<Profile[]>('get_profiles')
                  if (token !== bootToken || profiles.find(item => item.active)?.id !== profile) {
                    toast.close(key)
                    return
                  }
                  if (!await preinstall.allowPolicyVersions(versions, profile)) {
                    toast(i18next.t('plugins.authorize_failed'), { variant: 'danger', description: preinstall.error })
                    return
                  }
                  toast.close(key)
                  if (token === bootToken)
                    await this.restart()
                }
                catch (error) {
                  toast(i18next.t('plugins.authorize_failed'), { variant: 'danger', description: String(error) })
                }
                finally {
                  granting = false
                }
              },
            },
          })
          policyApprovalKey = key
        }
      }
    },

    prepareInstall(title: string) {
      this.status = 'installing'
      this.installer = { ...initialInstaller, title }
    },

    fail(message: string, logs?: string[], pluginConflictHint?: string, inotifyLimitHint?: string, keepServiceRunning = false, patchLayerHint?: string, heapOomHint?: string) {
      this.errorMsg = message
      this.errorLogs = logs ?? []
      this.pluginConflictHint = pluginConflictHint ?? ''
      this.inotifyLimitHint = inotifyLimitHint ?? ''
      this.patchLayerHint = patchLayerHint ?? ''
      this.heapOomHint = heapOomHint ?? ''
      this.status = 'error'
      this.serviceRunning = keepServiceRunning
    },

    async switchProfile(id: string) {
      if (this.busyAction)
        return
      this.busyAction = 'start'
      ++bootToken
      this.resetFrontendPreload()
      const healthy = this.serviceHealthy
      this.serviceHealthy = false
      try {
        await invoke<Profile>('set_active_profile', { id })
      }
      catch (error) {
        this.serviceHealthy = healthy
        this.busyAction = null
        throw error
      }
      try {
        this.serviceRunning = false
        void hooks['config.dialog.hidden'].trigger()
        await this.boot()
      }
      finally {
        this.busyAction = null
      }
    },

    restart(): Promise<void> {
      return restartFlight.run(async () => {
        if (this.busyAction)
          return
        this.busyAction = 'restart'
        ++bootToken
        this.resetFrontendPreload()

        this.serviceHealthy = false
        this.iframeLoaded = false
        this.iframeError = false

        recovery.hide()
        try {
          void hooks['config.dialog.hidden'].trigger()
          await invoke('shutdown_harness')
        }
        catch (err) {
          console.error('[Harness] shutdown during restart failed:', err)
        }
        this.serviceRunning = false
        try {
          await this.boot()
        }
        finally {
          this.busyAction = null
        }
      })
    },

    // issue #525: 后端同时隔离解析不了的补丁层（含作用于所有档案的 home 层）。
    async enterSafeMode() {
      await this.runPatchRepair('safe-mode', async () => {
        const report = await invoke<PatchQuarantineReport>('enter_safe_mode')
        notifyPatchQuarantine(report)
      })
    },

    async quarantineBrokenPatchLayers() {
      await this.runPatchRepair('quarantine', async () => {
        const report = await invoke<PatchQuarantineReport>('quarantine_broken_patch_layers')
        notifyPatchQuarantine(report)
      })
    },

    async stripUnresolvedPatchEntries() {
      await this.runPatchRepair('strip', async () => {
        const report = await invoke<PatchEntryStripReport>('strip_unresolved_patch_entries')
        notifyPatchEntryStrip(report)
      })
    },

    async runPatchRepair(action: PatchRepairAction, repair: () => Promise<void>) {
      if (this.busyAction)
        return
      ++bootToken
      this.resetFrontendPreload()
      this.busyAction = 'repair'
      try {
        await repair()
      }
      catch (err) {
        console.error(PATCH_REPAIR_LOG[action], err)
        const error = await attachStartupDiagnostics(err)
        this.fail(
          error.message,
          error.logs,
          error.pluginConflictHint,
          error.inotifyLimitHint,
          undefined,
          error.patchLayerHint,
        )
        return
      }
      finally {
        this.busyAction = null
      }
      if (action === 'safe-mode')
        await this.start()
      else
        await this.restart()
    },

    async shutdown() {
      if (this.busyAction)
        return
      this.busyAction = 'shutdown'
      ++bootToken
      this.resetFrontendPreload()

      void hooks['config.dialog.hidden'].trigger()
      try {
        await invoke('shutdown_harness')
      }
      catch (err) {
        console.error('[Harness] shutdown failed:', err)
      }
      finally {
        this.busyAction = null
      }
      this.serviceRunning = false
      this.status = 'error'
      this.errorMsg = i18next.t('ui.stopped')
      this.errorLogs = []
      this.pluginConflictHint = ''
      this.inotifyLimitHint = ''
      this.patchLayerHint = ''
      this.heapOomHint = ''
      recovery.reset()
    },

    async start() {
      if (this.busyAction)
        return
      this.busyAction = 'start'
      try {
        await this.boot()
      }
      finally {
        this.busyAction = null
      }
    },

    async openBrowser() {
      if (this.busyAction)
        return
      this.busyAction = 'openBrowser'
      try {
        await invoke('open_in_browser')
      }
      catch (err) {
        console.error('[Harness] open in browser failed:', err)
      }
      finally {
        this.busyAction = null
      }
    },
  },
})

// issue #705: 只看帧内自报，超时即转错误界面，别把浏览器错误页当成加载成功。
let iframeWatch: { loaded: boolean, timer: ReturnType<typeof setTimeout> } | null = null
harness.$subscribe(() => {
  const { status, serviceHealthy, iframeLoaded, iframeError } = harness.$state
  const watching = status === 'ready' && serviceHealthy && !harness.iframeAlive && !iframeError
  const context = {
    status,
    serviceHealthy,
    serviceRunning: harness.serviceRunning,
    iframeKey: harness.iframeKey,
    iframeLoaded,
    iframeAlive: harness.iframeAlive,
    iframeError,
  }
  if (!watching) {
    if (iframeWatch) {
      console.warn('[Harness] iframe watchdog cancelled', context)
      clearTimeout(iframeWatch.timer)
      iframeWatch = null
    }
    return
  }

  if (iframeWatch?.loaded === iframeLoaded)
    return
  if (iframeWatch) {
    clearTimeout(iframeWatch.timer)
  }
  const timeoutMs = iframeLoaded ? IFRAME_FRAME_GRACE_TIMEOUT : IFRAME_LOAD_TIMEOUT
  console.warn('[Harness] iframe watchdog armed', { ...context, timeoutMs })
  iframeWatch = {
    loaded: iframeLoaded,
    timer: setTimeout(() => {
      iframeWatch = null
      console.warn('[Harness] iframe watchdog expired', {
        status: harness.status,
        serviceHealthy: harness.serviceHealthy,
        serviceRunning: harness.serviceRunning,
        iframeKey: harness.iframeKey,
        iframeLoaded: harness.iframeLoaded,
        iframeAlive: harness.iframeAlive,
        iframeError: harness.iframeError,
        timeoutMs,
      })
      harness.iframeLoaded = false
      harness.iframeError = true
    }, timeoutMs),
  }
})
