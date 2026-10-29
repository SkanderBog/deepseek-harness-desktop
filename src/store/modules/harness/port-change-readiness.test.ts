import { beforeEach, describe, expect, it, vi } from 'vitest'

const state = vi.hoisted(() => ({
  urls: [] as string[],
  polls: [] as Array<Record<string, unknown>>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'get_runtime_info')
      return { service_url: state.urls.shift() ?? 'http://127.0.0.1:3080' }
    return undefined
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@/config/client', () => ({ queryClient: { invalidateQueries: vi.fn() } }))
vi.mock('../harness-updater', () => ({ harnessUpdater: { checkForUpdate: vi.fn() } }))
vi.mock('./utils', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./utils')>()
  return {
    ...actual,
    pollHarnessReadiness: vi.fn(async () => state.polls.shift() ?? { healthy: true, notOwned: false }),
  }
})

const { harness } = await import('./index')
const { pollHarnessReadiness } = await import('./utils')

beforeEach(() => {
  state.urls = []
  state.polls = []
  vi.mocked(pollHarnessReadiness).mockClear()
  Object.assign(harness, {
    serviceUrl: 'http://127.0.0.1:3080',
    iframeSrc: 'http://127.0.0.1:3080/?dshDesktop',
    frontendBootHtml: null,
  })
})

describe('就绪提交遇到端口改写', () => {
  it('上一轮探测落在旧端口时按新地址重新探测后再提交', async () => {
    state.urls = ['http://127.0.0.1:3081', 'http://127.0.0.1:3081']
    state.polls = [{ healthy: true, notOwned: false, bootOrigin: 'http://127.0.0.1:3081', bootHtml: '<new/>' }]

    await harness.completeReadiness(0, {
      healthy: true,
      notOwned: false,
      bootOrigin: 'http://127.0.0.1:3080',
      bootHtml: '<old/>',
    })

    expect(pollHarnessReadiness).toHaveBeenCalledTimes(1)
    expect(harness.serviceUrl).toBe('http://127.0.0.1:3081')
    expect(harness.frontendBootHtml).toBe('<new/>')
  })

  it('端口未变时沿用原探测结果，不重复探测', async () => {
    state.urls = ['http://127.0.0.1:3080']

    await harness.completeReadiness(0, {
      healthy: true,
      notOwned: false,
      bootOrigin: 'http://127.0.0.1:3080',
      bootHtml: '<boot/>',
    })

    expect(pollHarnessReadiness).not.toHaveBeenCalled()
    expect(harness.frontendBootHtml).toBe('<boot/>')
  })

  it('新端口的探测失败照常上报，不提交旧端口地址', async () => {
    state.urls = ['http://127.0.0.1:3081', 'http://127.0.0.1:3081']
    state.polls = [{ healthy: false, notOwned: false, phase: 'client-modules', reason: 'not ready', timeout: 'inactivity' }]

    await expect(harness.completeReadiness(0, {
      healthy: true,
      notOwned: false,
      bootOrigin: 'http://127.0.0.1:3080',
    })).rejects.toThrow()

    expect(harness.serviceUrl).toBe('http://127.0.0.1:3080')
  })
})
