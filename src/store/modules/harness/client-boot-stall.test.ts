import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@/config/client', () => ({ queryClient: { invalidateQueries: vi.fn() } }))
vi.mock('../harness-updater', () => ({ harnessUpdater: { checkForUpdate: vi.fn() } }))

const { harness } = await import('./index')

function mountGeneration(key: number) {
  Object.assign(harness, {
    iframeKey: key,
    iframeAliveKey: key,
    iframeBootReadyKey: null,
    iframeLoaded: true,
    iframeError: false,
  })
}

beforeEach(() => mountGeneration(7))

afterEach(() => {
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
