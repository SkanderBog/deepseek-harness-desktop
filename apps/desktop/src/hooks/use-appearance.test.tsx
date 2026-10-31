// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react'
import { useEffect } from 'react'
import { defineStore } from 'valtio-define'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useAppearance } from './use-appearance'

const mocks = vi.hoisted(() => ({
  setting: {} as any,
  platform: 'linux',
  setEffects: vi.fn(() => Promise.resolve()),
  clearEffects: vi.fn(() => Promise.resolve()),
  emit: vi.fn(),
  listeners: new Map<string, (event: { event: string, id: number, payload: unknown }) => void>(),
}))
vi.mock('@/store', () => ({ store: { get setting() {
  return mocks.setting
} } }))
vi.mock('tauri-runtime-electron/api/window', () => ({
  Effect: { Acrylic: 'acrylic', Mica: 'mica', UnderWindowBackground: 'underWindowBackground' },
  EffectState: { FollowsWindowActiveState: 'followsWindowActiveState' },
  getCurrentWindow: () => ({ label: 'appearance-window', setEffects: mocks.setEffects, clearEffects: mocks.clearEffects }),
}))
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: (target: unknown, event: string, payload?: unknown) => {
    expect(target).toEqual({ kind: 'Window', label: 'appearance-window' })
    return mocks.emit(event, payload)
  },
}))
vi.mock('@tauri-apps/plugin-os', () => ({ type: () => mocks.platform }))
vi.mock('./use-dsh-style', () => ({ useDshStyle: () => [{ colorScheme: 'dark' }] }))
vi.mock('./use-listen', () => ({
  useListen: (event: string, handler: (event: { event: string, id: number, payload: unknown }) => void) => {
    useEffect(() => {
      mocks.listeners.set(event, handler)
      return () => { mocks.listeners.delete(event) }
    }, [event, handler])
  },
}))

afterEach(() => {
  cleanup()
  document.body.replaceChildren()
  document.head.replaceChildren()
  delete (window as any).__DSH_TRANSPARENT__
  mocks.platform = 'linux'
  vi.clearAllMocks()
  vi.restoreAllMocks()
})

function setup(transparent: boolean, hydrated = true) {
  ;(window as any).__DSH_TRANSPARENT__ = transparent
  mocks.setting = defineStore({ state: () => ({ appearance: { palette: 'nord', terminal: false, opacity: 70 }, hydrated }) })
  mocks.emit.mockClear()
  mocks.listeners.clear()
  const hook = renderHook(() => useAppearance())
  function dispatch(event: string) {
    act(() => mocks.listeners.get(event)?.({ event, id: 1, payload: undefined }))
  }
  return {
    emit: mocks.emit,
    hook,
    ready: () => dispatch('dsh://appearance:ready'),
  }
}

describe('desktop appearance projection', () => {
  it('keeps the canvas opaque until the native window has restarted with transparency enabled', () => {
    const { emit, ready, hook } = setup(false)
    ready()
    expect(emit.mock.calls.at(-1)?.[1]).toMatchObject({ appearance: { opacity: 100 } })
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:#343c4a!important')
  })

  it('sends the saved opacity to a transparent window and removes the Tauri event listener on unmount', () => {
    const { emit, ready, hook } = setup(true)
    ready()
    expect(emit.mock.calls.at(-1)?.[1]).toMatchObject({ appearance: { palette: 'nord', opacity: 70 } })
    expect(hook.result.current).toContain('color-mix(in srgb,#2e3440 70%,transparent)')
    hook.unmount()
    emit.mockClear()
    ready()
    expect(emit).not.toHaveBeenCalled()
  })

  it('projects the same blur into the shell and Tauri appearance event', async () => {
    const { emit, ready, hook } = setup(true)
    ready()
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true }
    })
    expect(emit.mock.calls.at(-1)?.[1]).toMatchObject({ appearance: { blur: true } })
    expect(hook.result.current).toContain('backdrop-filter:blur(16px)')
    expect(hook.result.current).toContain('-webkit-backdrop-filter:blur(16px)')
  })

  it('paints the shell bar with the fill the embedded sidebar column shows', async () => {
    const { hook } = setup(true)
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true }
    })
    expect(hook.result.current).toContain('--color-canvas:#2e3440')
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:color-mix(in srgb,#2e3440 70%,transparent)!important')
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: false, opacity: 70 }
    })
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:#343c4a!important')
  })

  it.each([
    [{ transparency: true, opacity: 70 }, true],
    [{ transparency: true, opacity: 70, sidebarOnly: true }, true],
    [{ transparency: false, opacity: 70 }, false],
    [{ transparency: true, opacity: 100 }, false],
  ])('fills startup pages with the same alpha the shell bar uses for %j', async (preferences, translucent) => {
    const { hook } = setup(true)
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, ...preferences }
    })
    if (translucent)
      expect(hook.result.current).toContain('--color-startup:color-mix(in srgb,#2e3440 70%,transparent)')
    else
      expect(hook.result.current).toContain('--color-startup:#2e3440')
  })

  it('serializes native window effects on supported transparent desktops', async () => {
    mocks.platform = 'windows'
    const { hook } = setup(true)
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true }
    })
    await vi.waitFor(() => expect(mocks.setEffects).toHaveBeenLastCalledWith({
      effects: ['acrylic', 'mica', 'underWindowBackground'],
      state: 'followsWindowActiveState',
    }))

    mocks.clearEffects.mockClear()
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 100, blur: true }
    })
    await vi.waitFor(() => expect(mocks.setEffects).toHaveBeenCalledTimes(2))
    expect(mocks.clearEffects).not.toHaveBeenCalled()

    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: false }
    })
    await vi.waitFor(() => expect(mocks.clearEffects).toHaveBeenCalled())
    expect(mocks.setEffects.mock.invocationCallOrder.at(-1)).toBeLessThan(mocks.clearEffects.mock.invocationCallOrder.at(-1)!)
    hook.unmount()
  })

  it('does not request unsupported Linux window effects', async () => {
    mocks.platform = 'linux'
    setup(true)
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true }
    })
    expect(mocks.setEffects).not.toHaveBeenCalled()
    expect(mocks.clearEffects).not.toHaveBeenCalled()
  })

  it('immediately restores an opaque canvas when native transparency is disabled', async () => {
    const { emit, ready, hook } = setup(true)
    ready()
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, opacity: 70, transparency: false }
    })
    expect(emit.mock.calls.at(-1)?.[1]).toMatchObject({ appearance: { transparency: false, opacity: 100 } })
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:#343c4a!important')
  })

  it('keeps a transparent default palette opaque at 100 percent', async () => {
    const { hook, ready, emit } = setup(true)
    ready()
    await act(async () => {
      mocks.setting.appearance = { palette: 'default', terminal: false, transparency: true, opacity: 100, blur: true }
    })
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:#1b1b1c!important')
    expect(hook.result.current).not.toContain('backdrop-filter:blur')
    expect(emit.mock.calls.at(-1)?.[1]).toEqual({ appearance: expect.objectContaining({ palette: 'default', transparency: true, opacity: 100 }) })
  })

  it('keeps the settings dialog opaque with the content-area option', async () => {
    const { hook } = setup(true)
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true, sidebarOnly: true }
    })
    expect(hook.result.current).toContain('[data-testid="dsh-config-dialog"]{background:#343c4a!important}')
  })

  it('does not clear the startup effect before native settings hydrate', async () => {
    mocks.platform = 'windows'
    setup(true, false)
    expect(mocks.setEffects).not.toHaveBeenCalled()
    expect(mocks.clearEffects).not.toHaveBeenCalled()
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', terminal: false, transparency: true, opacity: 70, blur: true }
      mocks.setting.hydrated = true
    })
    await vi.waitFor(() => expect(mocks.setEffects).toHaveBeenCalledOnce())
  })

  it('applies high-contrast borders to the shell and removes them when switching palettes', async () => {
    const { hook } = setup(false)
    await act(async () => {
      mocks.setting.appearance = { palette: 'github-high-contrast', opacity: 100 }
    })
    expect(hook.result.current).toContain('--color-canvas:#010409')
    expect(hook.result.current).toContain('--field-border:#b7bdc8')
    await act(async () => {
      mocks.setting.appearance = { palette: 'nord', opacity: 100 }
    })
    expect(hook.result.current).not.toContain('--field-border:')
  })

  it('accepts appearance handshakes from the Tauri event listener', () => {
    const { emit, ready } = setup(true)
    ready()
    expect(emit).toHaveBeenCalledWith('dsh://appearance', { appearance: expect.objectContaining({ palette: 'nord' }) })
  })

  it('does no message work on unrelated renders and restores opaque styling on reset', async () => {
    const { emit, ready, hook } = setup(true)
    ready()
    emit.mockClear()
    hook.rerender()
    expect(emit).not.toHaveBeenCalled()
    await act(async () => {
      mocks.setting.appearance = { palette: 'default', terminal: false, opacity: 100 }
    })
    expect(emit.mock.calls.at(-1)?.[1]).toMatchObject({ appearance: { palette: 'default', opacity: 100 } })
    expect(hook.result.current).toContain('[data-testid="dsh-navbar-root"]{background:#1b1b1c!important')
  })
})
