// @vitest-environment jsdom
import type { NavbarProps } from './navbar'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { Webview } from './webview'

const mocks = vi.hoisted(() => ({
  navbar: vi.fn<(props: NavbarProps) => void>(),
  shortcuts: { rows: [] },
  setShortcuts: vi.fn(),
  harness: { status: 'ready', serviceHealthy: true, serviceRunning: true, webviewKey: 0, webviewLoaded: false, webviewAlive: false, webviewError: false, startupPhase: 'process-boot', busyAction: null },
  recovery: { recovery: { required: false } },
  emit: vi.fn().mockResolvedValue(undefined),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  action: null as null | ((event: { actionId?: string, inputValue?: string, extra?: { tag?: string } }) => void),
  clicked: null as null | ((event: { data?: { tag?: string } }) => Promise<void>),
  window: {
    label: 'window-2',
    isMinimized: vi.fn().mockResolvedValue(false),
    isVisible: vi.fn().mockResolvedValue(true),
    isFocused: vi.fn().mockResolvedValue(true),
    unminimize: vi.fn().mockResolvedValue(undefined),
    show: vi.fn().mockResolvedValue(undefined),
    setFocus: vi.fn().mockResolvedValue(undefined),
    onFocusChanged: vi.fn().mockResolvedValue(vi.fn()),
    onResized: vi.fn().mockResolvedValue(vi.fn()),
  },
}))

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => mocks.window }))
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: (target: unknown, event: string, payload?: unknown) => {
    expect(target).toEqual({ kind: 'Window', label: 'window-2' })
    return mocks.emit(event, payload)
  },
}))
vi.mock('@/hooks/use-notification-action', () => ({ useNotificationAction: (handler: typeof mocks.action) => {
  mocks.action = handler
} }))
vi.mock('@/hooks/use-notification-clicked', () => ({ useNotificationClicked: (handler: typeof mocks.clicked) => {
  mocks.clicked = handler
} }))
vi.mock('@tauri-apps/plugin-os', () => ({ type: () => 'windows' }))
vi.mock('valtio-define', () => ({ useStore: (value: unknown) => value }))
vi.mock('@/store', () => ({ store: { harness: mocks.harness, recovery: mocks.recovery } }))
vi.mock('@/hooks/use-appearance', () => ({ useAppearance: () => '' }))
vi.mock('@overlastic/react', () => ({ useOverlay: () => vi.fn() }))
vi.mock('@/ui/dialog/config', () => ({ ConfigDialog: () => null }))
vi.mock('@/hooks/use-dsh-style', () => ({ useDshStyle: () => [{}] }))
vi.mock('@/hooks/use-dsh-shortcuts', () => ({
  DSH_VIEW_COMMANDS: [],
  shortcutHint: vi.fn(),
  useDshShortcuts: () => [mocks.shortcuts, mocks.setShortcuts],
}))
vi.mock('@/hooks/use-listen', () => ({ useListen: (event: string, handler: (event: { payload: unknown }) => void) => {
  mocks.listeners.set(event, handler)
} }))
vi.mock('@/ui/plugin/recovery', () => ({ Recovery: () => null }))
vi.mock('./setup', () => ({ Setup: () => null }))
vi.mock('./setup-preinstall', () => ({ PreinstallSetup: () => null }))
vi.mock('./navbar', () => ({
  Navbar: (props: NavbarProps) => {
    mocks.navbar(props)
    return null
  },
}))
vi.mock('./harness-webview', () => ({
  HarnessWebview: ({ srcOverride }: { srcOverride?: string | null }) => (
    <div data-src={srcOverride ?? 'http://127.0.0.1:3081'} title="Harness" />
  ),
}))

it('logs the selected shell branch and deduplicates unrelated rerenders without exposing remote URLs', () => {
  window.location.hash = ''
  const view = render(<Webview />)
  expect(console.warn).toHaveBeenLastCalledWith('[Webview] shell view', expect.objectContaining({
    branch: 'webview',
    status: 'ready',
    remoteMode: false,
    serviceHealthy: true,
    webviewKey: 0,
    webviewLoaded: false,
    webviewAlive: false,
    webviewError: false,
  }))
  view.rerender(<Webview />)
  expect(console.warn).toHaveBeenCalledTimes(1)
  mocks.harness.status = 'error'
  view.rerender(<Webview />)
  expect(console.warn).toHaveBeenLastCalledWith('[Webview] shell view', expect.objectContaining({ branch: 'setup' }))
  mocks.recovery.recovery.required = true
  view.rerender(<Webview />)
  expect(console.warn).toHaveBeenLastCalledWith('[Webview] shell view', expect.objectContaining({ branch: 'recovery' }))
  mocks.harness.status = 'preinstall'
  view.rerender(<Webview />)
  expect(console.warn).toHaveBeenLastCalledWith('[Webview] shell view', expect.objectContaining({ branch: 'preinstall' }))
  mocks.harness.status = 'ready'
  mocks.harness.serviceHealthy = false
  view.rerender(<Webview />)
  act(() => navbar().onRemoteChange('https://example.test/?token=secret', null))
  expect(console.warn).toHaveBeenLastCalledWith('[Webview] shell view', expect.objectContaining({ branch: 'webview', remoteMode: true, serviceHealthy: false }))
  expect(JSON.stringify(vi.mocked(console.warn).mock.calls)).not.toContain('secret')
  expect(console.warn).toHaveBeenCalledTimes(6)
})

function navbar() {
  return mocks.navbar.mock.lastCall![0]
}

beforeEach(() => {
  vi.useFakeTimers()
  window.location.hash = ''
  Object.assign(mocks.harness, { status: 'ready', serviceHealthy: true })
  mocks.recovery.recovery.required = false
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  mocks.navbar.mockClear()
  mocks.setShortcuts.mockClear()
  vi.clearAllMocks()
  mocks.listeners.clear()
  mocks.window.isMinimized.mockResolvedValue(false)
  mocks.window.isVisible.mockResolvedValue(true)
  mocks.window.isFocused.mockResolvedValue(true)
})

afterEach(() => {
  cleanup()
  window.location.hash = ''
  vi.useRealTimers()
  vi.restoreAllMocks()
})

it('sends sidebar and settings actions through the registered Tauri event contracts', () => {
  render(<Webview />)
  act(() => {
    navbar().onToggleSidebar?.()
    navbar().onOpenMachineManager?.()
    navbar().onOpenRemoteAccess?.()
  })
  expect(mocks.emit).toHaveBeenCalledWith('dsh://sidebar:toggle', undefined)
  expect(mocks.emit).toHaveBeenCalledWith('dsh://settings:open', { section: 'dsh-tauri-remote', tab: 'machines' })
  expect(mocks.emit).toHaveBeenCalledWith('dsh://settings:open', { section: 'dsh-tauri-remote', tab: 'access' })
})

it('returns to local content before opening remote settings after plugin readiness', () => {
  render(<Webview />)
  act(() => navbar().onRemoteChange('https://remote.test', null))
  act(() => navbar().onOpenMachineManager?.())
  expect(mocks.emit).not.toHaveBeenCalledWith('dsh://settings:open', expect.anything())
  act(() => mocks.listeners.get('dsh://appearance:ready')!({ payload: undefined }))
  expect(mocks.emit).toHaveBeenCalledWith('dsh://settings:open', { section: 'dsh-tauri-remote', tab: 'machines' })
})

it('coalesces duplicate notification actions without losing the response text', () => {
  render(<Webview />)
  act(() => {
    mocks.action!({ actionId: 'reply', inputValue: 'answer', extra: { tag: 'session-tag' } })
    mocks.action!({ actionId: 'reply', extra: { tag: 'session-tag' } })
    vi.advanceTimersByTime(250)
  })
  expect(mocks.emit.mock.calls.filter(([event]) => event === 'dsh://notification-clicked')).toEqual([
    ['dsh://notification-clicked', { tag: 'session-tag', action: 'reply', inputValue: 'answer' }],
  ])
  expect(mocks.window.show).not.toHaveBeenCalled()
})

it('activates a minimized window before delivering a native notification click', async () => {
  mocks.window.isMinimized.mockResolvedValue(true)
  render(<Webview />)
  await act(async () => {
    await mocks.clicked!({ data: { tag: 'session-tag' } })
  })
  expect(mocks.window.unminimize).toHaveBeenCalledOnce()
  expect(mocks.window.show).toHaveBeenCalledOnce()
  expect(mocks.window.setFocus).toHaveBeenCalledOnce()
  expect(mocks.emit).toHaveBeenCalledWith('dsh://notification-clicked', { tag: 'session-tag' })
})
