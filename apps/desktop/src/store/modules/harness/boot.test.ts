import { invoke } from '@tauri-apps/api/core'
import i18next from 'i18next'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { toast } from '@/utils/toast'
import { PLUGIN_INACTIVITY_TIMEOUT } from './constants'
import { harness } from './store'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@/config/client', () => ({ queryClient: { invalidateQueries: vi.fn() } }))
vi.mock('@/config/hooks', () => ({ hooks: {} }))
vi.mock('@/utils/toast', () => ({ toast: Object.assign(vi.fn(() => 'approval'), { close: vi.fn() }) }))
vi.mock('../harness-updater', () => ({ harnessUpdater: { checkForUpdate: vi.fn() } }))
vi.mock('../plugins', () => import('../plugins/utils'))
vi.mock('../preinstall', () => ({ preinstall: {} }))
vi.mock('../recovery', () => ({ recovery: { clear: vi.fn(), reset: vi.fn(), reviewStartupRecovery: vi.fn() } }))
vi.mock('../setting', () => ({ setting: { $state: { installed: true }, refresh: vi.fn() } }))

let ensure: () => Promise<void>
let cancel: () => Promise<void>

beforeEach(async () => {
  vi.useFakeTimers()
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  vi.spyOn(console, 'error').mockImplementation(() => {})
  await i18next.init({
    lng: 'en',
    keySeparator: false,
    resources: { en: { translation: { 'errors.startup_failed': 'Startup failed: {{reason}}' } } },
  })
  ensure = async () => {}
  cancel = async () => {}
  vi.mocked(invoke).mockImplementation(async (command) => {
    switch (command) {
      case 'get_runtime_info': return { service_url: 'http://127.0.0.1:3080' }
      case 'runtime_ready': return true
      case 'get_profiles': return [{ id: 'tauri', active: true }]
      case 'ensure_internal_plugins': return ensure()
      case 'cancel_internal_plugins': return cancel()
      case 'get_preinstall_pending': return false
      case 'launch_harness':
      case 'harness_ownership': return
      case 'proxy_health_check': return 'healthy'
      case 'read_service_logs': return ''
      default: throw new Error(`Unexpected command: ${command}`)
    }
  })
  Object.assign(harness, { serviceRunning: false, serviceHealthy: false, busyAction: null })
})

afterEach(async () => {
  harness.serviceHealthy = false
  await vi.advanceTimersByTimeAsync(0)
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

it.each([
  'INTERNAL_PLUGIN_SOURCE_MISSING: dsh-tauri host entry missing',
  'INTERNAL_PLUGIN_INSTALL_FAILED: pnpm add failed',
  'PNPM_INSTALL_FAILED: exit code 1',
  'PNPM_PROBE_LOCK_TIMEOUT: install lock busy',
  'INTERNAL_PLUGIN_INSTALL_TIMEOUT: process tree recycled',
  'INTERNAL_PLUGIN_INSTALL_CANCELLED: process tree recycled',
])('launches Harness despite the ordinary plugin failure %s', async (message) => {
  ensure = () => Promise.reject(message)
  await harness.boot()
  expect(invoke).toHaveBeenCalledWith('launch_harness')
  expect(harness.serviceHealthy).toBe(true)
  expect(harness.status).toBe('ready')
})

it.each([
  'PNPM_PROBE_TREE_CLEANUP_FAILED: process tree exit unconfirmed',
  'PNPM_PROBE_CLEANUP_TIMEOUT: cleanup did not finish',
  'PLUGIN_PROCESS_COORDINATOR_DROPPED: cleanup unconfirmed',
  'PREINSTALL_WAIT: wait task failed',
  'INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT: owner active',
  'INTERNAL_PLUGIN_ENSURE_DROPPED: coordinator ended',
])('blocks Harness while cleanup is unconfirmed: %s', async (message) => {
  ensure = () => Promise.reject(message)
  await harness.boot()
  expect(invoke).not.toHaveBeenCalledWith('launch_harness')
  expect(harness.status).toBe('error')
  expect(harness.errorMsg).toContain(message)
})

it('launches Harness after a timed-out plugin task confirms cancellation', async () => {
  ensure = () => new Promise(() => {})
  let confirm!: () => void
  cancel = () => new Promise<void>((resolve) => {
    confirm = resolve
  })
  const boot = harness.boot()
  await vi.advanceTimersByTimeAsync(PLUGIN_INACTIVITY_TIMEOUT)
  expect(invoke).toHaveBeenCalledWith('cancel_internal_plugins')
  expect(invoke).not.toHaveBeenCalledWith('launch_harness')
  confirm()
  await boot
  expect(invoke).toHaveBeenCalledWith('launch_harness')
  expect(harness.serviceHealthy).toBe(true)
})

it('blocks Harness when cancellation cannot confirm process cleanup', async () => {
  ensure = () => new Promise(() => {})
  cancel = () => Promise.reject(new Error('PLUGIN_PROCESS_CLEANUP_FAILED: owner active'))
  const boot = harness.boot()
  await vi.advanceTimersByTimeAsync(PLUGIN_INACTIVITY_TIMEOUT)
  await boot
  expect(invoke).toHaveBeenCalledWith('cancel_internal_plugins')
  expect(invoke).not.toHaveBeenCalledWith('launch_harness')
  expect(harness.status).toBe('error')
  expect(harness.errorMsg).toContain('PLUGIN_PROCESS_CLEANUP_FAILED: owner active')
})

it('keeps the version approval prompt while launching without the blocked plugin', async () => {
  ensure = () => Promise.reject(new Error('INTERNAL_PLUGIN_INSTALL_FAILED: PLUGIN_POLICY_BLOCKED:[{"name":"dsh-tauri","version":"1.2.3"}]'))
  await harness.boot()
  expect(invoke).toHaveBeenCalledWith('launch_harness')
  expect(harness.serviceHealthy).toBe(true)
  expect(toast).toHaveBeenCalledWith('plugins.blocked_policy_title', expect.objectContaining({
    variant: 'warning',
    actionProps: expect.objectContaining({ children: 'buttons.authorize' }),
  }))
})

it('retains failure for a malformed version policy refusal', async () => {
  ensure = () => Promise.reject(new Error('PLUGIN_POLICY_BLOCKED:[{"name":"dsh-tauri","version":"invalid"}]'))
  await harness.boot()
  expect(invoke).not.toHaveBeenCalledWith('launch_harness')
  expect(harness.status).toBe('error')
  expect(toast).not.toHaveBeenCalled()
})
