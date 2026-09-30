import { beforeEach, describe, expect, it, vi } from 'vitest'

interface ToastCallOptions {
  timeout?: number
  isLoading?: boolean
  onClose?: (reason: string) => void
  actionProps?: { onPress?: () => void }
}

const { invoke, restart, toast } = vi.hoisted(() => {
  const toastFn = vi.fn((_message: string, _options?: ToastCallOptions) => 'toast-key')
  return {
    invoke: vi.fn(),
    restart: vi.fn(),
    toast: Object.assign(toastFn, { close: vi.fn(), update: vi.fn(), clear: vi.fn(), isActive: vi.fn(() => true) }),
  }
})

vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('../src/store/modules/harness', () => ({ harness: { restart } }))
vi.mock('@/utils/toast', () => ({ toast }))

const { plugins } = await import('../src/store/modules/plugins')

const RUNTIME = { toast: false, restartOnSettle: false }

beforeEach(() => {
  invoke.mockReset()
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
  plugins.queueResults = []
})

describe('plugins manager settle', () => {
  it('asks before restarting instead of restarting a succeeding group on its own', async () => {
    invoke.mockResolvedValue(undefined)

    const results = await plugins.enqueue('install', ['a', 'b'], { toast: true, restartOnSettle: true })

    expect(results.every(result => result.ok)).toBe(true)
    expect(restart).not.toHaveBeenCalled()
    const prompt = toast.mock.calls.find(call => call[1]?.actionProps !== undefined)
    expect(prompt?.[1]?.timeout).toBe(0)
    prompt?.[1]?.actionProps?.onPress?.()
    expect(toast.close).toHaveBeenCalled()
    expect(restart).toHaveBeenCalledTimes(1)
  })

  it('folds the restart button into the single result toast instead of adding a second one', async () => {
    invoke.mockResolvedValue(undefined)

    await plugins.enqueue('install', ['a'], { toast: true, restartOnSettle: true })

    const toasts = toast.mock.calls.filter(call => call[1]?.isLoading !== true)
    expect(toasts).toHaveLength(1)
    expect(toasts[0][1]?.timeout).toBe(0)
    toasts[0][1]?.actionProps?.onPress?.()
    expect(toast.close).toHaveBeenCalled()
    expect(restart).toHaveBeenCalledTimes(1)
  })

  it('leaves the harness running when restartOnSettle is disabled', async () => {
    invoke.mockResolvedValue(undefined)

    await plugins.enqueue('install', ['a'], RUNTIME)

    expect(restart).not.toHaveBeenCalled()
    expect(toast.mock.calls.some(call => call[1]?.actionProps !== undefined)).toBe(false)
  })

  it('does not restart when every process of the group failed', async () => {
    invoke.mockRejectedValue('REGISTRY_DOWN: registry unreachable')

    const results = await plugins.enqueue('install', ['a'], { toast: true, restartOnSettle: true })

    expect(results.map(result => [result.ok, result.code, result.reason])).toEqual([
      [false, 'REGISTRY_DOWN', undefined],
    ])
    expect(restart).not.toHaveBeenCalled()
    expect(toast.mock.calls.some(call => call[1]?.actionProps !== undefined)).toBe(false)
  })

  it('does not restart when every process was skipped before reaching the host', async () => {
    invoke.mockResolvedValue(undefined)
    plugins.setInstalled([])

    const results = await plugins.enqueue('uninstall', ['a'], { toast: true, restartOnSettle: true })

    expect(results.map(result => [result.ok, result.reason])).toEqual([[false, 'already-absent']])
    expect(invoke).not.toHaveBeenCalled()
    expect(restart).not.toHaveBeenCalled()
    expect(toast.mock.calls.some(call => call[1]?.actionProps !== undefined)).toBe(false)
  })

  it('emits completed, error and allcompleted around a failing group', async () => {
    invoke.mockRejectedValue('REGISTRY_DOWN: registry unreachable')
    const seen: string[] = []
    const offs = [
      plugins.on('completed', () => seen.push('completed')),
      plugins.on('error', () => seen.push('error')),
      plugins.on('allcompleted', () => seen.push('allcompleted')),
    ]

    try {
      await plugins.enqueue('install', ['a'], RUNTIME)
    }
    finally {
      offs.forEach(off => off())
    }

    expect(seen).toEqual(['completed', 'error', 'allcompleted'])
  })

  it('emits allcompleted only once the queue is drained', async () => {
    invoke.mockResolvedValue(undefined)
    const seen: string[] = []
    const off = plugins.on('allcompleted', payload => seen.push(String(payload.length)))

    try {
      await Promise.all([
        plugins.enqueue('install', ['a'], RUNTIME),
        plugins.enqueue('install', ['b'], RUNTIME),
      ])
    }
    finally {
      off()
    }

    expect(seen).toEqual(['1'])
  })
})
