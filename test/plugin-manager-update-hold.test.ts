import i18next from 'i18next'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { resources } from '../src/i18n/index.resource'

interface ToastCallOptions {
  timeout?: number
  variant?: string
  isLoading?: boolean
  description?: string
  actionProps?: { children?: string }
  onClose?: (reason: string) => void
}

const { invoke, restart, toast } = vi.hoisted(() => {
  let index = 0
  const toastFn = vi.fn((_message: string, _options?: ToastCallOptions): string => `toast-${++index}`)
  return {
    invoke: vi.fn(),
    restart: vi.fn(),
    toast: Object.assign(toastFn, { close: vi.fn(), update: vi.fn(), clear: vi.fn(), isActive: vi.fn(() => true) }),
  }
})

vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('../src/store/modules/harness', () => ({ harness: { restart } }))
vi.mock('@/utils/toast', () => ({ toast }))

// 与 src/i18n/index.ts 相同的扁平 key 约定：断言的标题/副标题取真实文案
await i18next.init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: resources as unknown as Record<string, { translation: Record<string, string> }>,
  interpolation: { escapeValue: false },
  keySeparator: false,
  nsSeparator: false,
  initAsync: false,
})

const { plugins } = await import('../src/store/modules/plugins')

const RUNTIME = { toast: true, restartOnSettle: false }
const PINNED_DESC = 'No new version to authorise: the profile pins the source (a catalog/git/link or exact spec), so `--latest` cannot move past it. The plugin itself is fine.'

function holdError(payload: Record<string, unknown>): Error {
  return new Error(`PLUGIN_UPDATE_NO_CHANGE: ${JSON.stringify(payload)}`)
}

function approvalToast() {
  return toast.mock.calls.find(call => call[1]?.onClose !== undefined)
}

function resultToasts() {
  return toast.mock.calls.filter(call => call[1]?.isLoading !== true)
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  toast.mockClear()
  toast.close.mockClear()
  restart.mockClear()
  plugins.groups = []
  plugins.processes = []
  plugins.logs = []
  plugins.activeGroupId = null
  plugins.cancelling = false
  plugins.presenterCount = 0
  plugins.installedSource = []
  plugins.installedLoaded = false
  plugins.progressKey = null
  plugins.progressDetail = ''
})

describe('plugins manager update hold', () => {
  it('keeps asking for authorisation while the exact version can still be exempted', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'update_dsh_plugins')
        throw holdError({ name: 'aaa', version: '0.1.0', latest: '0.1.174', retryable: true })
      return undefined
    })
    plugins.attachPresenter()

    const done = plugins.enqueue('upgrade', ['aaa'], RUNTIME)
    await vi.waitFor(() => expect(plugins.pendingApprovals).toHaveLength(1))

    expect(plugins.pendingApprovals[0].refusal).toEqual({
      kind: 'update-hold',
      versions: [{ name: 'aaa', version: '0.1.174' }],
      retryable: true,
    })

    const approval = approvalToast()
    expect(approval?.[0]).toBe('aaa was not upgraded')
    expect(approval?.[1]?.timeout).toBe(0)
    expect(approval?.[1]?.actionProps?.children).toBe('Authorise')

    await plugins.cancel()
    await done
  })

  it('settles a pinned source as a no-op instead of offering a useless authorisation', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'update_dsh_plugins')
        throw holdError({ name: 'aaa', version: '0.1.0', latest: '0.1.174', retryable: false })
      return undefined
    })
    plugins.attachPresenter()

    const results = await plugins.enqueue('upgrade', ['aaa'], RUNTIME)

    expect(results.map(result => [result.process.name, result.ok, result.reason])).toEqual([
      ['aaa', false, 'update-hold'],
    ])
    expect(plugins.pendingApprovals).toHaveLength(0)
    expect(approvalToast()).toBeUndefined()

    const toasts = resultToasts()
    expect(toasts).toHaveLength(1)
    expect(toasts[0][0]).toBe('aaa was not upgraded')
    expect(toasts[0][1]?.description).toBe(PINNED_DESC)
    expect(toasts[0][1]?.variant).toBeUndefined()
    expect(toasts[0][1]?.timeout).toBeUndefined()
  })

  it('does not report a plain failure for the pinned source', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'update_dsh_plugins')
        throw holdError({ name: 'aaa', version: '0.1.0', latest: '0.1.174', retryable: false })
      return undefined
    })
    plugins.attachPresenter()

    await plugins.enqueue('upgrade', ['aaa'], RUNTIME)

    expect(toast.mock.calls.some(call => call[1]?.variant === 'danger')).toBe(false)
  })

  it('settles every target when the payload carries no candidate version', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'update_dsh_plugins')
        throw holdError({ name: 'aaa', version: '0.1.0', latest: null })
      return undefined
    })
    plugins.attachPresenter()

    const done = plugins.enqueue('upgrade', ['aaa', 'bbb'], RUNTIME)
    const guarded = await Promise.race([
      done,
      new Promise(resolve => setTimeout(resolve, 500, 'hung')),
    ])

    expect(guarded).not.toBe('hung')
    const results = await done
    expect(results.map(result => [result.process.name, result.ok, result.reason])).toEqual([
      ['aaa', false, 'update-hold'],
      ['bbb', false, 'update-hold'],
    ])
    expect(approvalToast()).toBeUndefined()
  })

  it('reports the untouched members of a pinned batch as upgraded', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command !== 'update_dsh_plugins')
        return undefined
      throw holdError({ name: 'bbb', version: '0.2.0', latest: '0.2.1', retryable: false })
    })
    plugins.attachPresenter()

    const results = await plugins.enqueue('upgrade', ['aaa', 'bbb'], RUNTIME)

    expect(invoke.mock.calls.filter(call => call[0] === 'update_dsh_plugins')).toHaveLength(1)
    expect(results.map(result => [result.process.name, result.ok, result.reason])).toEqual([
      ['aaa', true, undefined],
      ['bbb', false, 'update-hold'],
    ])
  })
})
