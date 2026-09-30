import i18next from 'i18next'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { resources } from '../src/i18n/index.resource'

interface ToastCallOptions {
  timeout?: number
  variant?: string
  isLoading?: boolean
  description?: string
  actionProps?: { children?: string, onPress?: () => void }
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

const RUNTIME = { toast: true, restartOnSettle: true }

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
  plugins.installedSource = []
  plugins.installedLoaded = false
  plugins.progressKey = null
  plugins.progressDetail = ''
  plugins.queueResults = []
})

describe('plugins manager queue summary', () => {
  it('reports the whole queue with a single toast once it drains', async () => {
    await Promise.all([
      plugins.enqueue('install', ['a'], RUNTIME),
      plugins.enqueue('upgrade', ['b'], RUNTIME),
      plugins.enqueue('uninstall', ['c'], RUNTIME),
    ])

    const toasts = resultToasts()
    expect(toasts).toHaveLength(1)
    expect(toasts[0][0]).toBe('Plugin operations finished')
    expect(toasts[0][1]?.description).toBe('3 succeeded')
    expect(toasts[0][1]?.timeout).toBe(0)
    toasts[0][1]?.actionProps?.onPress?.()
    expect(toast.close).toHaveBeenCalled()
    expect(restart).toHaveBeenCalledTimes(1)
    expect(plugins.queueResults).toEqual([])
  })

  it('keeps reporting a lone result on its own instead of as a queue summary', async () => {
    await plugins.enqueue('install', ['a'], { toast: true, restartOnSettle: false })

    const toasts = resultToasts()
    expect(toasts).toHaveLength(1)
    expect(toasts[0][0]).toBe('Plugin a installed')
  })

  it('counts successes and failures of the whole queue in the same summary', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'update_dsh_plugins')
        throw new Error('REGISTRY_DOWN: registry unreachable')
      return undefined
    })

    await Promise.all([
      plugins.enqueue('install', ['a'], { toast: true, restartOnSettle: false }),
      plugins.enqueue('upgrade', ['b'], { toast: true, restartOnSettle: false }),
    ])

    const toasts = resultToasts()
    expect(toasts).toHaveLength(1)
    expect(toasts[0][0]).toBe('Plugin operations finished')
    expect(toasts[0][1]?.description).toBe('1 succeeded · 1 failed')
    expect(toasts[0][1]?.variant).toBe('danger')
    expect(toasts[0][1]?.timeout).toBeUndefined()
    expect(toasts[0][1]?.actionProps).toBeUndefined()
  })
})
