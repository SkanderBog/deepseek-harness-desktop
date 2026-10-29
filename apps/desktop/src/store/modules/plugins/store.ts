import type { PluginsManagerEvent, PluginsManagerEventMap } from './events'
import type {
  BlockedRefusal,
  Plugin,
  PluginGroup,
  PluginProcess,
  PluginProcessReason,
  PluginProcessResult,
  PluginProcessType,
  PluginRef,
  PluginSearchProblem,
  PluginSearchResult,
  PluginsManagerRuntime,
  PluginsState,
} from './types'
import type { DshPlugin } from '@/types'
import type { ToastCloseReason } from '@/utils/toast'
import { invoke } from '@tauri-apps/api/core'
import i18next from 'i18next'
import { defineStore } from 'valtio-define'
import { toast } from '@/utils/toast'
import { onPluginsManagerEvent, triggerPluginsManagerEvent } from './events'
import { enrichInstalled, normalizeRef, normalizeRefs, parseBlockedRefusal, parseUpdateFailures, refusalNames } from './utils'

const LOG_LIMIT = 200
const MAX_ATTEMPTS = 8

const COMMANDS: Record<PluginProcessType, string> = {
  install: 'install_plugin_specs',
  upgrade: 'update_dsh_plugins',
  uninstall: 'remove_dsh_plugins',
  disable: 'disable_dsh_plugin',
  enable: 'enable_dsh_plugin',
}

const PROGRESS_ONE: Record<PluginProcessType, string> = {
  install: 'plugins.progress_install_one',
  upgrade: 'plugins.progress_upgrade_one',
  uninstall: 'plugins.progress_uninstall_one',
  disable: 'plugins.progress_disable_one',
  enable: 'plugins.progress_enable_one',
}

const PROGRESS_MANY: Record<PluginProcessType, string> = {
  install: 'plugins.progress_install_many',
  upgrade: 'plugins.progress_upgrade_many',
  uninstall: 'plugins.progress_uninstall_many',
  disable: 'plugins.progress_disable_many',
  enable: 'plugins.progress_enable_many',
}

const RESULT_SUCCESS: Record<PluginProcessType, string> = {
  install: 'plugins.result_install_success',
  upgrade: 'plugins.result_upgrade_success',
  uninstall: 'plugins.result_uninstall_success',
  disable: 'plugins.result_disable_success',
  enable: 'plugins.result_enable_success',
}

const RESULT_FAILED: Record<PluginProcessType, string> = {
  install: 'plugins.result_install_failed',
  upgrade: 'plugins.result_upgrade_failed',
  uninstall: 'plugins.result_uninstall_failed',
  disable: 'plugins.result_disable_failed',
  enable: 'plugins.result_enable_failed',
}

const BLOCK_TITLE: Record<BlockedRefusal['kind'], string> = {
  'incompatible': 'plugins.blocked_incompatible_title',
  'policy': 'plugins.blocked_policy_title',
  'update-hold': 'plugins.hold_title',
}

const BLOCK_DESC: Record<BlockedRefusal['kind'], string> = {
  'incompatible': 'plugins.blocked_incompatible_desc',
  'policy': 'plugins.blocked_policy_desc',
  'update-hold': 'plugins.hold_desc',
}

const NOOP_MESSAGE: Record<PluginProcessType, string> = {
  install: 'plugins.already_absent',
  upgrade: 'plugins.no_change',
  uninstall: 'plugins.already_absent',
  disable: 'plugins.no_change',
  enable: 'plugins.no_change',
}

interface PluginInspectPayload {
  spec: string
  name?: string
  version?: string
  compatible?: boolean | null
  peers?: Record<string, string>
  problem?: PluginSearchProblem
}

function nextGroupId(type: PluginProcessType): string {
  return `${type}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function errorCode(message: string): string | undefined {
  return /^([A-Z][A-Z0-9_]+):/.exec(message)?.[1]
}

export const plugins = defineStore({
  state: (): PluginsState => ({
    groups: [],
    processes: [],
    logs: [],
    activeGroupId: null,
    cancelling: false,
    installedSource: [],
    installedLoaded: false,
    progressKey: null,
    progressDetail: '',
    queueResults: [],
  }),
  getters: {
    installed(): Plugin[] {
      return enrichInstalled(this.installedSource)
    },
    pendingApprovals(): PluginProcess[] {
      return this.processes.filter(process => process.status === 'unauthorized')
    },
  },
  actions: {
    pushLog(level: 'info' | 'error', message: string, groupId?: string, processId?: string): void {
      this.logs = [...this.logs, { at: Date.now(), level, message, groupId, processId }].slice(-LOG_LIMIT)
    },

    setInstalled(list: DshPlugin[]): void {
      this.installedSource = list
      this.installedLoaded = true
    },

    async refresh(): Promise<DshPlugin[]> {
      const list = await invoke<DshPlugin[]>('refresh_plugin_updates')
      this.setInstalled(list)
      return list
    },

    on<K extends PluginsManagerEvent>(
      event: K,
      handler: (payload: PluginsManagerEventMap[K]) => void,
    ): () => void {
      return onPluginsManagerEvent(event, handler)
    },

    enqueue(
      type: PluginProcessType,
      refs: PluginRef | PluginRef[],
      options: PluginsManagerRuntime,
    ): Promise<PluginProcessResult[]> {
      const normalized = normalizeRefs(refs)
      const groupId = nextGroupId(type)
      const created: PluginProcess[] = normalized.map((ref, index) => ({
        id: `${groupId}#${index}`,
        groupId,
        type,
        status: 'pending',
        spec: ref.spec,
        name: ref.name,
        ...(ref.version === undefined ? {} : { version: ref.version }),
      }))
      let resolveDone: (results: PluginProcessResult[]) => void = () => {}
      const done = new Promise<PluginProcessResult[]>((resolve) => {
        resolveDone = resolve
      })
      const group: PluginGroup = {
        id: groupId,
        type,
        status: 'pending',
        processIds: created.map(process => process.id),
        attempt: 0,
        pending: created.map(process => process.id),
        results: [],
        done,
        resolveDone,
        options,
      }
      this.processes = [...this.processes, ...created]
      this.groups = [...this.groups, group]
      this.pushLog('info', i18next.t('plugins.log_enqueued', { count: created.length }), groupId)
      this.syncProgress()
      void this.drain()
      return done
    },

    async drain(): Promise<void> {
      if (this.activeGroupId !== null)
        return
      const group = this.groups.find(item => item.status === 'pending')
      if (!group)
        return
      this.activeGroupId = group.id
      group.status = 'active'
      try {
        await this.runGroup(group)
      }
      finally {
        this.activeGroupId = null
        void this.drain()
      }
    },

    async runGroup(group: PluginGroup): Promise<void> {
      this.syncProgress()
      while (group.pending.length > 0) {
        const targets = group.pending
          .map(id => this.processes.find(process => process.id === id))
          .filter((process): process is PluginProcess => process !== undefined && process.status === 'pending')
        if (this.cancelling)
          break
        if (targets.length === 0) {
          this.hideProgress()
          await new Promise<void>((resolve) => {
            group.resume = resolve
          })
          group.resume = undefined
          this.syncProgress()
          continue
        }
        if (group.attempt >= MAX_ATTEMPTS) {
          targets.forEach((process) => {
            this.finish(group.id, process, {
              process,
              ok: false,
              error: i18next.t('plugins.retry_exhausted'),
              reason: 'retry-exhausted',
            })
          })
          break
        }
        const runnable: PluginProcess[] = []
        targets.forEach((process) => {
          const reason = this.precheck(process)
          if (reason === null) {
            runnable.push(process)
            return
          }
          this.finish(group.id, process, {
            process,
            ok: false,
            error: i18next.t(reason === 'not-installed' ? 'plugins.not_installed' : NOOP_MESSAGE[process.type], {
              name: process.name,
            }),
            reason,
          })
        })
        if (runnable.length === 0)
          continue
        runnable.forEach((process) => {
          process.status = 'running'
        })
        const blocked = await this.submit(group, runnable)
        if (blocked.length > 0) {
          this.hideProgress()
          await new Promise<void>((resolve) => {
            group.resume = resolve
          })
          group.resume = undefined
          this.syncProgress()
          continue
        }
        runnable.forEach(process => this.detach(process.id))
      }
      this.settle(group)
    },

    precheck(process: PluginProcess): PluginProcessReason | null {
      if (process.type === 'install' || process.type === 'upgrade' || !this.installedLoaded)
        return null
      const installed = this.installedSource.find(item => item.id === process.name || item.name === process.name)
      if (!installed)
        return process.type === 'uninstall' ? 'already-absent' : 'not-installed'
      if (process.type === 'disable' && installed.disabled)
        return 'already-absent'
      if (process.type === 'enable' && !installed.disabled)
        return 'already-absent'
      return null
    },

    async submit(group: PluginGroup, targets: PluginProcess[]): Promise<PluginProcess[]> {
      group.attempt += 1
      if (group.type === 'disable' || group.type === 'enable') {
        const settled = await Promise.allSettled(
          targets.map(process =>
            invoke<void>(COMMANDS[group.type], { id: process.name }),
          ),
        )
        const blocked: PluginProcess[] = []
        settled.forEach((outcome, index) => {
          const process = targets[index]
          if (outcome.status === 'fulfilled') {
            this.finish(group.id, process, { process, ok: true })
            return
          }
          const message = errorMessage(outcome.reason)
          const refusal = parseBlockedRefusal(message)
          if (refusal !== null && refusalNames(refusal).size > 0) {
            this.block(group, process, refusal, group.options.toast)
            blocked.push(process)
            return
          }
          this.finish(group.id, process, { process, ok: false, error: message, code: errorCode(message) })
        })
        return blocked
      }
      try {
        await invoke<void>(COMMANDS[group.type], {
          ...(group.type === 'install'
            ? { specs: targets.map(process => process.spec) }
            : group.type === 'upgrade'
              ? { ids: targets.map(process => process.spec) }
              : { ids: targets.map(process => process.name) }),
        })
        targets.forEach(process => this.finish(group.id, process, { process, ok: true }))
        return []
      }
      catch (error) {
        const message = errorMessage(error)
        const refusal = parseBlockedRefusal(message)

        const failures = new Map(parseUpdateFailures(message).map(entry => [entry.name, entry.message]))
        const settleFailure = (process: PluginProcess): boolean => {
          const detail = failures.get(process.name)
          if (detail === undefined)
            return false
          this.finish(group.id, process, { process, ok: false, error: detail, code: errorCode(detail) })
          return true
        }

        if (refusal?.kind === 'update-hold') {
          const names = refusalNames(refusal)
          const known = new Set(refusal.versions.map(item => item.name))
          const blocked: PluginProcess[] = []
          targets.forEach((process) => {
            if (!group.pending.includes(process.id))
              return
            if (names.size > 0 && !names.has(process.name)) {
              if (settleFailure(process))
                return
              this.finish(group.id, process, { process, ok: true })
              return
            }
            if (names.size > 0 && refusal.retryableNames.includes(process.name)) {
              this.block(group, process, refusal, group.options.toast)
              blocked.push(process)
              return
            }
            this.finish(group.id, process, {
              process,
              ok: false,
              error: i18next.t(
                known.has(process.name) ? 'plugins.hold_pinned_desc' : 'plugins.hold_no_target',
              ),
            })
          })
          return blocked
        }
        if (refusal !== null && refusalNames(refusal).size > 0) {
          const names = refusalNames(refusal)
          const blocked: PluginProcess[] = []
          targets.forEach((process) => {
            if (!group.pending.includes(process.id))
              return
            if (names.has(process.name)) {
              this.block(group, process, refusal, group.options.toast)
              blocked.push(process)
              return
            }
            if (settleFailure(process))
              return
            process.status = 'pending'
          })

          if (blocked.length > 0)
            return blocked
        }

        if (failures.size > 0) {
          targets.forEach((process) => {
            if (!group.pending.includes(process.id))
              return
            if (settleFailure(process))
              return
            this.finish(group.id, process, { process, ok: true })
          })
          return []
        }
        targets.forEach(process =>
          this.finish(group.id, process, {
            process,
            ok: false,
            error: message,
            code: errorCode(message),
          }),
        )
        return []
      }
    },

    block(group: PluginGroup, process: PluginProcess, refusal: BlockedRefusal, presenter: boolean): void {
      process.status = 'unauthorized'
      process.refusal = refusal
      this.pushLog('error', i18next.t('plugins.not_authorized', { name: process.name }), group.id, process.id)
      if (!presenter)
        return
      const versions = refusal.versions.map(item => `${item.name}@${item.version}`).join('、')
      process.approvalKey = toast(i18next.t(BLOCK_TITLE[refusal.kind], { name: process.name }), {
        variant: refusal.kind === 'incompatible' ? 'danger' : 'warning',
        timeout: 0,

        sticky: true,
        description: i18next.t(BLOCK_DESC[refusal.kind], { blocked: versions }),
        actionProps: {
          children: i18next.t('buttons.authorize'),
          onPress: () => {
            void this.approve(process.spec)
          },
        },
        onClose: (reason: ToastCloseReason) => {
          if (reason !== 'dismissed')
            return
          void this.reject(process.spec)
        },
      })
    },

    finish(groupId: string, process: PluginProcess, result: PluginProcessResult): void {
      const group = this.groups.find(item => item.id === groupId)

      if (group === undefined || !group.pending.includes(process.id)) {
        this.detach(process.id)
        return
      }
      group.pending = group.pending.filter(id => id !== process.id)
      group.results = [...group.results, result]
      if (!result.ok) {
        this.pushLog('error', result.error ?? i18next.t('plugins.action_failed'), groupId, process.id)
      }
      this.detach(process.id)
    },

    detach(processId: string): void {
      this.processes = this.processes.filter(process => process.id !== processId)
    },

    settle(group: PluginGroup): void {
      const results = group.results
      const failed = results.filter(result => !result.ok)
      group.status = 'settled'
      this.groups = this.groups.filter(item => item.id !== group.id)
      group.processIds.forEach(id => this.detach(id))
      this.syncProgress()
      group.resolveDone(results)
      triggerPluginsManagerEvent('completed', results)
      if (failed.length > 0)
        triggerPluginsManagerEvent('error', failed)
      this.presentResults(group, results)
      if (this.groups.length === 0)
        triggerPluginsManagerEvent('allcompleted', results)
    },

    progressTargets(): PluginProcess[] {
      return this.processes.filter(process => process.status !== 'unauthorized')
    },

    progressTitle(processes: PluginProcess[]): string {
      const first = processes[0]
      if (processes.length >= 2) {
        const types = new Set(processes.map(process => process.type))
        return types.size === 1
          ? i18next.t(PROGRESS_MANY[first.type], { count: processes.length })
          : i18next.t('plugins.progress_many', { count: processes.length })
      }
      return i18next.t(PROGRESS_ONE[first.type], { name: first.name })
    },

    syncProgress(): void {
      const processes = this.progressTargets()

      const wanted = processes.length > 0
        && this.groups.some(group => group.options.toast)
      if (!wanted) {
        this.hideProgress()
        return
      }
      const title = this.progressTitle(processes)
      const description = this.progressDetail === '' ? undefined : this.progressDetail

      const current = this.progressKey
      if (current === null || !toast.isActive(current))
        this.progressKey = toast(title, { timeout: 0, isLoading: true, description })
      else
        toast.update(current, { title, isLoading: true, description })
      const key = this.progressKey
      processes.forEach((process) => {
        process.progressKey = key ?? undefined
      })
    },

    hideProgress(): void {
      if (this.progressKey !== null) {
        toast.close(this.progressKey)
        this.progressKey = null
      }
      this.progressDetail = ''
      this.processes.forEach((process) => {
        process.progressKey = undefined
      })
    },

    setProgressDetail(line: string): void {
      const current = this.progressKey
      if (current === null || !toast.isActive(current))
        return
      this.progressDetail = line
      toast.update(current, { description: line })
    },

    presentResults(group: PluginGroup, results: PluginProcessResult[]): void {
      if (!group.options.toast)
        return
      this.queueResults.push(...results)
      if (this.groups.length > 0)
        return
      const pending = this.queueResults
      this.queueResults = []
      this.presentQueueResults(pending)
    },

    presentQueueResults(results: PluginProcessResult[]): void {
      if (results.length === 0)
        return
      const succeeded = results.filter(result => result.ok).length

      const noop = results.filter(result => result.reason === 'already-absent').length
      const failed = results.filter(
        result => !result.ok
          && result.reason !== 'already-absent'
          && result.reason !== 'cancelled'
          && result.reason !== 'rejected',
      ).length
      if (results.length === 1) {
        const [result] = results

        if (result.reason === 'cancelled' || result.reason === 'rejected')
          return
        const title = i18next.t(
          result.ok ? RESULT_SUCCESS[result.process.type] : RESULT_FAILED[result.process.type],
          { name: result.process.name },
        )
        toast(
          title,
          result.ok ? { variant: 'default' } : { variant: 'danger', description: result.error },
        )
        return
      }
      const parts = [
        succeeded > 0 ? i18next.t('plugins.queue_summary_success', { count: succeeded }) : '',
        failed > 0 ? i18next.t('plugins.queue_summary_failed', { count: failed }) : '',
        noop > 0 ? i18next.t('plugins.queue_summary_skipped', { count: noop }) : '',
      ].filter(part => part !== '')

      if (parts.length === 0)
        return
      toast(i18next.t('plugins.queue_summary'), {
        description: parts.join(' · '),
        variant: failed > 0 ? 'danger' : 'default',
      })
    },

    async approve(refs?: PluginRef | PluginRef[]): Promise<PluginProcessResult[]> {
      const group = this.activeGroup()
      if (!group)
        return []
      const targets = this.approvalTargets(group, refs)
      if (targets.length === 0)
        return group.done
      for (const process of targets) {
        const refusal = process.refusal
        if (!refusal)
          continue
        try {
          await invoke<void>(refusal.kind === 'incompatible' ? 'allow_plugin_versions' : 'allow_plugin_policy_versions', {
            versions: refusal.versions,
          })
        }
        catch (error) {
          const message = errorMessage(error)
          this.pushLog('error', message, group.id, process.id)
          if (group.options.toast) {
            toast(i18next.t('plugins.authorize_failed'), {})
          }
          continue
        }
        if (process.approvalKey !== undefined) {
          toast.close(process.approvalKey)
          process.approvalKey = undefined
        }
        process.refusal = undefined
        process.status = 'pending'
        triggerPluginsManagerEvent('approve', process)
      }
      group.resume?.()
      return group.done
    },

    async reject(ref: PluginRef): Promise<PluginProcessResult[]> {
      const normalized = normalizeRef(ref)
      const group = this.activeGroup()
      if (!group)
        return []
      const process = this.approvalTargets(group, normalized.spec).find(
        item => item.name === normalized.name || item.spec === normalized.spec,
      )
      if (!process)
        return group.done
      if (process.approvalKey !== undefined) {
        toast.close(process.approvalKey)
        process.approvalKey = undefined
      }
      this.finish(group.id, process, {
        process,
        ok: false,
        error: i18next.t('plugins.rejected'),
        reason: 'rejected',
      })
      group.resume?.()
      return group.done
    },

    async cancel(): Promise<void> {
      const group = this.activeGroup()
      if (!group || this.cancelling)
        return
      this.cancelling = true
      try {
        await invoke<void>('cancel_plugin_processes')
      }
      catch (error) {
        this.pushLog('error', errorMessage(error), group.id)
      }
      const remaining = group.pending
        .map(id => this.processes.find(process => process.id === id))
        .filter((process): process is PluginProcess => process !== undefined)
      remaining.forEach((process) => {
        if (process.approvalKey !== undefined) {
          toast.close(process.approvalKey)
          process.approvalKey = undefined
        }
        this.finish(group.id, process, {
          process,
          ok: false,
          error: i18next.t('plugins.cancelled'),
          reason: 'cancelled',
        })
      })
      this.cancelling = false
      group.resume?.()
    },

    async search(refs: PluginRef | PluginRef[], options?: { dsh?: string }): Promise<PluginSearchResult[]> {
      const normalized = normalizeRefs(refs)
      const specs = normalized.map(item => item.spec)
      try {
        const inspected = await invoke<PluginInspectPayload[]>('inspect_plugin_specs', {
          specs,
          dsh: options?.dsh ?? null,
        })
        return inspected.map(item => ({
          spec: item.spec,
          name: item.name,
          version: item.version,
          compatible: item.compatible ?? null,
          peers: item.peers,
          problem: item.problem,
        }))
      }
      catch (error) {
        this.pushLog('error', errorMessage(error))
        return specs.map(spec => ({ spec, compatible: null, problem: 'network' as const }))
      }
    },

    activeGroup(): PluginGroup | undefined {
      return this.groups.find(item => item.id === this.activeGroupId)
    },

    approvalTargets(group: PluginGroup, refs?: PluginRef | PluginRef[]): PluginProcess[] {
      const unauthorized = group.pending
        .map(id => this.processes.find(process => process.id === id))
        .filter((process): process is PluginProcess => process !== undefined && process.status === 'unauthorized')
      if (refs === undefined)
        return unauthorized
      const normalized = normalizeRefs(refs)
      const names = new Set(normalized.map(item => item.name))
      return unauthorized.filter(process => names.has(process.name))
    },
  },
})
