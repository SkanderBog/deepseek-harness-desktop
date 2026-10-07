import type { HostContext } from 'dsh-tauri'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { apply, inject, packagedSkillsDir } from './apply'
import { clearHostRuntime, getCurrentHostInstance, providerRuntime } from './config/runtime'

vi.mock('./routes', () => ({ routes: vi.fn(() => () => {}) }))
vi.mock('./service/provider', () => ({ provider: { start: vi.fn().mockResolvedValue(undefined) } }))
vi.mock('./service/profile', () => ({ profile: { resolve: () => null, peek: (name: string) => `/profiles/${name}` } }))

const { provider } = await import('./service/provider')
const { routes } = await import('./routes')

afterEach(() => {
  clearHostRuntime()
  vi.clearAllMocks()
})

describe('apply provider lifecycle', () => {
  it('starts packaged skills only when the registered effect runs and returns runtime cleanup', async () => {
    const effects = new Map<string, () => (() => void)>()
    const host = { marker: 'host' }
    const ctx = {
      inject: vi.fn((_names: string[], callback: (context: unknown) => unknown) => callback(host)),
      effect: vi.fn((effect: () => (() => void), label: string) => effects.set(label, effect)),
    }
    apply(ctx as unknown as HostContext, { profile: 'chosen' })
    expect(ctx.inject).toHaveBeenCalledWith(inject, expect.any(Function))
    expect(provider.start).not.toHaveBeenCalled()
    expect(providerRuntime.disposed).toBe(false)
    expect(getCurrentHostInstance()).toBe(host)
    const start = effects.get('dsh-tauri-extension: skill provider')!
    const dispose = start()
    expect(provider.start).toHaveBeenCalledExactlyOnceWith(packagedSkillsDir())
    expect(dispose).toBe(clearHostRuntime)
    effects.get('dsh-tauri-extension: routes')!()
    expect(routes).toHaveBeenCalledWith(host, {
      profileDirPath: '/profiles/chosen',
      remountProvider: expect.any(Function),
      hotReload: expect.any(Function),
    })
    expect(vi.mocked(routes).mock.calls[0][1].hotReload()).toBe(false)
    await vi.mocked(routes).mock.calls[0][1].remountProvider()
    expect(provider.start).toHaveBeenCalledTimes(2)
    dispose()
    expect(providerRuntime.disposed).toBe(true)
    expect(providerRuntime.fiber).toBeUndefined()
    expect(() => getCurrentHostInstance()).toThrow()
  })

  it('宿主挂载 hmr 服务时报告可热加载', () => {
    const effects = new Map<string, () => (() => void)>()
    const host = { marker: 'host', get: (name: string) => (name === 'hmr' ? { runExclusive: async () => {} } : undefined) }
    const ctx = {
      inject: vi.fn((_names: string[], callback: (context: unknown) => unknown) => callback(host)),
      effect: vi.fn((effect: () => (() => void), label: string) => effects.set(label, effect)),
    }
    apply(ctx as unknown as HostContext)
    expect(providerRuntime.disposed).toBe(false)
    effects.get('dsh-tauri-extension: routes')!()
    expect(vi.mocked(routes).mock.calls[0][1].hotReload()).toBe(true)
  })
})
