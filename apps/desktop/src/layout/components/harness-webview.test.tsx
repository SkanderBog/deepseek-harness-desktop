// @vitest-environment jsdom
import { runInNewContext } from 'node:vm'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { HarnessWebview } from './harness-webview'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args: Record<string, unknown>) => Promise<unknown>>(),
  zoom: vi.fn().mockResolvedValue(undefined),
  harness: {
    serviceHealthy: true,
    webviewSrc: 'http://127.0.0.1:3081',
    webviewKey: 0,
    showWebviewError: false,
    startupStatusKey: 'status.loading_plugins',
    markWebviewLoaded: vi.fn(),
    markWebviewError: vi.fn(),
    refreshWebview: vi.fn(),
    markWebviewAlive: vi.fn(),
    markWebviewLeaving: vi.fn(),
    markWebviewBootReady: vi.fn(),
  },
  setting: { zoom_factor: 1.25 },
  setZoomFactor: vi.fn(),
  executeJavaScript: vi.fn<(script: string) => Promise<boolean>>(),
  listeners: new Map<string, () => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('valtio-define', () => ({ useStore: (value: unknown) => value }))
vi.mock('@/store', () => ({ store: { harness: mocks.harness, setting: { zoom: mocks.zoom, get zoom_factor() {
  return mocks.setting.zoom_factor
} } } }))
vi.mock('@/hooks/use-listen', () => ({ useListen: (event: string, handler: () => void) => {
  mocks.listeners.set(event, handler)
} }))
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
vi.mock('./loadable', () => ({ Loadable: ({ errorMsg }: { errorMsg?: string }) => <div role="status">{errorMsg ?? 'Loading'}</div> }))

beforeEach(() => {
  vi.clearAllMocks()
  mocks.listeners.clear()
  Object.assign(mocks.harness, { serviceHealthy: true, webviewKey: 0, showWebviewError: false })
  mocks.setting.zoom_factor = 1.25
  mocks.invoke.mockImplementation(async command => command === 'prepare_harness_webview' ? 'guest-local' : undefined)
  Object.defineProperty(HTMLElement.prototype, 'setZoomFactor', { configurable: true, value: mocks.setZoomFactor })
  Object.defineProperty(HTMLElement.prototype, 'executeJavaScript', { configurable: true, value: mocks.executeJavaScript })
  mocks.executeJavaScript.mockResolvedValue(false)
})
afterEach(async () => {
  cleanup()
  await act(async () => {})
  Reflect.deleteProperty(HTMLElement.prototype, 'setZoomFactor')
  Reflect.deleteProperty(HTMLElement.prototype, 'executeJavaScript')
  Reflect.deleteProperty(window, '__DSH_NAVIGATION__')
  vi.restoreAllMocks()
})

it('keeps one native registration through StrictMode and a reload during pending preparation', async () => {
  const active = new Map<string, string>()
  let sequence = 0
  let duplicates = 0
  let finishPrepare!: () => void
  let finishRelease: (() => void) | undefined
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === 'prepare_harness_webview') {
      const url = String(args.url)
      if (active.has(url)) {
        duplicates++
        throw new Error('invalid or duplicate guest registration')
      }
      const label = `guest-${++sequence}`
      active.set(url, label)
      await new Promise<void>((resolve) => {
        finishPrepare = resolve
      })
      return label
    }
    await new Promise<void>((resolve) => {
      finishRelease = resolve
    })
    for (const [url, label] of active) {
      if (label === args.label)
        active.delete(url)
    }
  })
  const view = render(<StrictMode><HarnessWebview /></StrictMode>)
  await waitFor(() => expect([...active.values()]).toEqual(['guest-1']))
  expect(duplicates).toBe(0)
  mocks.harness.webviewKey++
  view.rerender(<StrictMode><HarnessWebview /></StrictMode>)
  await act(async () => {})
  expect(duplicates).toBe(0)
  await act(async () => {
    finishPrepare()
  })
  await waitFor(() => expect(finishRelease).toBeTypeOf('function'))
  expect(screen.queryByTitle('Harness')).toBeNull()
  expect([...active.values()]).toEqual(['guest-1'])
  await act(async () => {
    finishRelease!()
  })
  await waitFor(() => expect([...active.values()]).toEqual(['guest-2']))
  await act(async () => {
    finishPrepare()
  })
  expect(screen.getByTitle('Harness').getAttribute('src')).toBe('http://127.0.0.1:3081')
  expect(duplicates).toBe(0)
  finishRelease = undefined
  view.unmount()
  await waitFor(() => expect(finishRelease).toBeTypeOf('function'))
  await act(async () => {
    finishRelease!()
  })
  expect(active.size).toBe(0)
})

it('waits for native registration before loading a guest and releases it on unmount', async () => {
  const navigation = { sessionId: 'session-existing' }
  Object.defineProperty(window, '__DSH_NAVIGATION__', { configurable: true, value: navigation })
  let prepared!: (label: string) => void
  mocks.invoke.mockImplementation(command => command === 'prepare_harness_webview'
    ? new Promise((resolve) => {
        prepared = resolve
      })
    : Promise.resolve())
  const view = render(<HarnessWebview />)
  expect(screen.queryByTitle('Harness')).toBeNull()
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('prepare_harness_webview', { url: 'http://127.0.0.1:3081', navigation }))
  await act(async () => {
    prepared('guest-registered')
  })
  expect(screen.getByTitle('Harness').getAttribute('src')).toBe('http://127.0.0.1:3081')
  view.unmount()
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('release_harness_webview', { label: 'guest-registered' }))
})

it('does not load a guest when native registration fails', async () => {
  mocks.invoke.mockRejectedValueOnce(new Error('registration denied'))
  render(<HarnessWebview />)
  expect(await screen.findByText('Error: registration denied')).not.toBeNull()
  expect(screen.queryByTitle('Harness')).toBeNull()
})

it('uses Electron DOM readiness for zoom and filters aborted or subframe loading errors', async () => {
  const view = render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  expect(guest.tagName).toBe('WEBVIEW')
  expect(mocks.setZoomFactor).not.toHaveBeenCalled()
  fireEvent(guest, new Event('dom-ready'))
  expect(mocks.setZoomFactor).toHaveBeenLastCalledWith(1.25)
  mocks.setting.zoom_factor = 1.5
  view.rerender(<HarnessWebview />)
  expect(mocks.setZoomFactor).toHaveBeenLastCalledWith(1.5)
  fireEvent(guest, new Event('did-finish-load'))
  expect(mocks.harness.markWebviewLoaded).toHaveBeenCalledOnce()
  for (const detail of [{ errorCode: -3, isMainFrame: true }, { errorCode: -2, isMainFrame: false }])
    fireEvent(guest, Object.assign(new Event('did-fail-load'), detail))
  expect(mocks.harness.markWebviewError).not.toHaveBeenCalled()
  fireEvent(guest, Object.assign(new Event('did-fail-load'), { errorCode: -2, isMainFrame: true }))
  expect(mocks.harness.markWebviewError).toHaveBeenCalledOnce()
})

it('confirms the current Harness guest without waiting for appearance readiness', async () => {
  mocks.executeJavaScript.mockImplementation(async (script) => {
    const guestWindow = {
      location: { origin: 'http://127.0.0.1:3081' },
      __DSH_BOOT__: { entries: [] },
      __TAURI_INTERNALS__: { metadata: { currentWebview: { label: 'guest-local' } } },
    }
    return runInNewContext(script, { window: guestWindow })
  })
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  await act(async () => {
    fireEvent(guest, new Event('did-finish-load'))
  })
  expect(mocks.harness.markWebviewAlive).toHaveBeenCalledOnce()
})

it.each([
  ['browser error page', undefined, 'guest-local', 'http://127.0.0.1:3081'],
  ['unregistered guest', { entries: [] }, 'other-guest', 'http://127.0.0.1:3081'],
  ['foreign document', { entries: [] }, 'guest-local', 'https://foreign.test'],
])('does not confirm a %s as the Harness document', async (_name, boot, label, origin) => {
  mocks.executeJavaScript.mockImplementation(async script => runInNewContext(script, { window: {
    location: { origin },
    __DSH_BOOT__: boot,
    __TAURI_INTERNALS__: { metadata: { currentWebview: { label } } },
  } }))
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  await act(async () => {
    fireEvent(guest, new Event('did-finish-load'))
  })
  expect(mocks.harness.markWebviewLoaded).toHaveBeenCalledOnce()
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('discards guest proof after main-frame navigation starts', async () => {
  let resolve!: (value: boolean) => void
  mocks.executeJavaScript.mockImplementation(() => new Promise((done) => {
    resolve = done
  }))
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  fireEvent(guest, new Event('did-finish-load'))
  fireEvent(guest, Object.assign(new Event('did-start-navigation'), { isMainFrame: true, isInPlace: false }))
  expect(mocks.harness.markWebviewLeaving).toHaveBeenCalledOnce()
  await act(async () => {
    resolve(true)
  })
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('discards old guest proof after a reload changes the generation', async () => {
  let resolve!: (value: boolean) => void
  mocks.executeJavaScript.mockImplementation(() => new Promise((done) => {
    resolve = done
  }))
  const view = render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  fireEvent(guest, new Event('did-finish-load'))
  mocks.harness.webviewKey++
  view.rerender(<HarnessWebview />)
  await act(async () => {
    resolve(true)
  })
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('keeps appearance readiness separate from guest document proof', async () => {
  render(<HarnessWebview />)
  await screen.findByTitle('Harness')
  act(() => {
    mocks.listeners.get('dsh://appearance:ready')!()
  })
  expect(mocks.harness.markWebviewBootReady).toHaveBeenCalledOnce()
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('keeps document proof through subframe and same-document navigation', async () => {
  mocks.executeJavaScript.mockResolvedValue(true)
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  await act(async () => {
    fireEvent(guest, new Event('did-finish-load'))
  })
  for (const detail of [{ isMainFrame: false, isInPlace: false }, { isMainFrame: true, isInPlace: true }])
    fireEvent(guest, Object.assign(new Event('did-start-navigation'), detail))
  expect(mocks.harness.markWebviewAlive).toHaveBeenCalledWith(0)
  expect(mocks.harness.markWebviewLeaving).not.toHaveBeenCalled()
})

it('revokes proof and rejects a pending probe when the renderer crashes', async () => {
  let resolve!: (value: boolean) => void
  mocks.executeJavaScript.mockImplementation(() => new Promise((done) => {
    resolve = done
  }))
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  fireEvent(guest, new Event('did-finish-load'))
  fireEvent(guest, new Event('render-process-gone'))
  await act(async () => {
    resolve(true)
  })
  expect(mocks.harness.markWebviewLeaving).toHaveBeenCalledOnce()
  expect(mocks.harness.markWebviewError).toHaveBeenCalledOnce()
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('keeps a rejected document probe unconfirmed', async () => {
  const error = new Error('guest evaluation failed')
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  mocks.executeJavaScript.mockRejectedValue(error)
  render(<HarnessWebview />)
  const guest = await screen.findByTitle('Harness')
  await act(async () => {
    fireEvent(guest, new Event('did-finish-load'))
  })
  expect(warn).toHaveBeenCalledWith('[Webview] failed to verify guest document:', error)
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
})

it('does not write local Harness health for a remote guest', async () => {
  render(<HarnessWebview srcOverride="http://127.0.0.1:4080" />)
  const guest = await screen.findByTitle('Harness')
  fireEvent(guest, new Event('did-finish-load'))
  fireEvent(guest, Object.assign(new Event('did-start-navigation'), { isMainFrame: true }))
  fireEvent(guest, new Event('render-process-gone'))
  act(() => {
    mocks.listeners.get('dsh://appearance:ready')!()
  })
  expect(mocks.executeJavaScript).not.toHaveBeenCalled()
  expect(mocks.harness.markWebviewLoaded).not.toHaveBeenCalled()
  expect(mocks.harness.markWebviewAlive).not.toHaveBeenCalled()
  expect(mocks.harness.markWebviewLeaving).not.toHaveBeenCalled()
  expect(mocks.harness.markWebviewError).not.toHaveBeenCalled()
  expect(mocks.harness.markWebviewBootReady).not.toHaveBeenCalled()
})

it('releases a stale registration when the selected URL changes', async () => {
  const pending = new Map<string, (label: string) => void>()
  mocks.invoke.mockImplementation((command, args) => command === 'prepare_harness_webview'
    ? new Promise((resolve) => { pending.set(String(args.url), resolve) })
    : Promise.resolve())
  const view = render(<HarnessWebview srcOverride="https://alpha.test" />)
  await waitFor(() => expect(pending.has('https://alpha.test')).toBe(true))
  view.rerender(<HarnessWebview srcOverride="https://beta.test" />)
  await act(async () => {
    pending.get('https://alpha.test')!('stale-guest')
  })
  expect(screen.queryByTitle('Harness')).toBeNull()
  expect(mocks.invoke).toHaveBeenCalledWith('release_harness_webview', { label: 'stale-guest' })
  await act(async () => {
    pending.get('https://beta.test')!('active-guest')
  })
  expect(screen.getByTitle('Harness').getAttribute('src')).toBe('https://beta.test')
})

it('registers a fresh guest when the harness requests a reload', async () => {
  const view = render(<HarnessWebview />)
  const previous = await screen.findByTitle('Harness')
  mocks.invoke.mockImplementation(async command => command === 'prepare_harness_webview' ? 'guest-reloaded' : undefined)
  mocks.harness.webviewKey++
  view.rerender(<HarnessWebview />)
  await waitFor(() => expect(screen.getByTitle('Harness')).not.toBe(previous))
  expect(mocks.invoke).toHaveBeenCalledWith('release_harness_webview', { label: 'guest-local' })
})
