import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@/config/client', () => ({ queryClient: { invalidateQueries: vi.fn() } }))
vi.mock('../harness-updater', () => ({ harnessUpdater: { checkForUpdate: vi.fn() } }))

const recoveryPoll = vi.hoisted(() => ({
  resolve: null as null | ((result: { healthy: boolean, notOwned: boolean }) => void),
}))
const gates = vi.hoisted(() => [] as Array<{ reset: () => void }>)
vi.mock('./readiness', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./readiness')>()
  class TrackedReloadGate extends actual.BoundedReloadGate {
    constructor(maxAttempts: number) {
      super(maxAttempts)
      gates.push(this)
    }
  }
  return { ...actual, BoundedReloadGate: TrackedReloadGate }
})
vi.mock('./utils', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./utils')>()
  return {
    ...actual,
    pollHarnessReadiness: vi.fn(() => new Promise((resolve) => {
      recoveryPoll.resolve = resolve
    })),
  }
})

const { harness } = await import('./index')

function mountGeneration(key: number) {
  Object.assign(harness, {
    iframeKey: key,
    iframeAliveKey: key,
    iframeBootReadyKey: null,
    iframeLoaded: true,
    iframeError: false,
    status: 'ready',
    serviceRunning: true,
  })
}

beforeEach(() => {
  gates.forEach(gate => gate.reset())
  mountGeneration(7)
})

afterEach(() => {
  recoveryPoll.resolve = null
  vi.useRealTimers()
})

describe('客户端 boot 完成确认', () => {
  it('只替当前这一代 iframe 背书', () => {
    harness.markIframeBootReady()
    expect(harness.iframeBootReady).toBe(true)

    harness.iframeKey++
    expect(harness.iframeBootReady).toBe(false)
  })

  it('重挂与帧内导航都会作废旧确认', async () => {
    vi.useFakeTimers()

    harness.markIframeBootReady()
    harness.refreshIframe()
    expect(harness.iframeBootReady).toBe(false)
    await vi.advanceTimersByTimeAsync(800)
    expect(harness.iframeKey).toBe(8)
    expect(harness.iframeBootReady).toBe(false)

    harness.markIframeAlive()
    harness.markIframeBootReady()
    expect(harness.iframeBootReady).toBe(true)
    harness.markIframeLeaving()
    expect(harness.iframeBootReady).toBe(false)
  })
})

describe('恢复探测的结论只作用于发起它的那份文档', () => {
  it('帧内导航后返回的失败结论不会把新文档判成启动失败', async () => {
    const recovery = harness.recoverIframeBoot()
    harness.markIframeLeaving()
    recoveryPoll.resolve?.({ healthy: false, notOwned: false })
    await recovery

    expect(harness.status).toBe('ready')
  })

  it('换帧后返回的就绪结论不会重挂新文档', async () => {
    vi.useFakeTimers()

    const recovery = harness.recoverIframeBoot()
    const replaced = harness.iframeKey + 1
    harness.iframeKey = replaced
    recoveryPoll.resolve?.({ healthy: true, notOwned: false })
    await recovery
    await vi.advanceTimersByTimeAsync(800)

    expect(harness.iframeKey).toBe(replaced)
  })

  it('探测期间帧内自报完成：迟到的失败结论不再把页面判成失败', async () => {
    const recovery = harness.recoverIframeBoot()
    harness.markIframeBootReady()
    recoveryPoll.resolve?.({ healthy: false, notOwned: false })
    await recovery

    expect(harness.status).toBe('ready')
    expect(harness.iframeBootReady).toBe(true)
  })

  it('探测通过且这一代仍待恢复时照旧重挂', async () => {
    vi.useFakeTimers()

    const recovery = harness.recoverIframeBoot()
    recoveryPoll.resolve?.({ healthy: true, notOwned: false })
    await recovery
    expect(harness.iframeKey).toBe(7)
    await vi.advanceTimersByTimeAsync(800)

    expect(harness.iframeKey).toBe(8)
  })
})
