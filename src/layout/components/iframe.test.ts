// @vitest-environment jsdom
import { act, cleanup, render } from '@testing-library/react'
import { createElement, createRef } from 'react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

const bridge = vi.hoisted(() => ({
  message: undefined as undefined | ((data: Record<string, unknown>) => void),
  post: vi.fn(),
  register: vi.fn(async (_types: unknown) => {}),
  send: vi.fn(async (_notification: unknown) => {}),
}))

vi.mock('@choochmeque/tauri-plugin-notifications-api', () => ({
  registerActionTypes: bridge.register,
  sendNotification: bridge.send,
}))
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
vi.mock('valtio-define', () => ({ useStore: (value: unknown) => value }))
vi.mock('@/config/client', () => ({ queryClient: {} }))
vi.mock('@/store', () => ({ store: {
  harness: { serviceHealthy: true, markIframeAlive: vi.fn(), markIframeLoaded: vi.fn() },
  setting: {},
} }))
vi.mock('@/hooks/use-iframe-message', () => ({
  useIframeMessage: (_ref: unknown, callback: typeof bridge.message) => {
    bridge.message = callback
  },
}))
vi.mock('@/hooks/use-iframe-post', () => ({ useIframePost: () => bridge.post }))
vi.mock('@/hooks/use-notification-action', () => ({ useNotificationAction: vi.fn() }))
vi.mock('@/hooks/use-notification-clicked', () => ({ useNotificationClicked: vi.fn() }))
vi.mock('@/hooks/use-dsh-style', () => ({ useDshStyle: () => [undefined, vi.fn()] }))
vi.mock('@/hooks/use-invoke-iframe', () => ({ useInvokeIframe: vi.fn() }))
vi.mock('@/hooks/use-sync-visibility', () => ({ useSyncVisibility: vi.fn() }))
vi.mock('@/hooks/use-zoom-factor', () => ({ useZoomFactor: vi.fn() }))
vi.mock('./loadable', () => ({ Loadable: () => null }))

const { Iframe } = await import('./iframe')

beforeEach(() => {
  vi.clearAllMocks()
  render(createElement(Iframe, { iframeRef: createRef<HTMLIFrameElement>() }))
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

it('registers background text input and a submit button before sending every question round', async () => {
  for (let round = 1; round <= 2; round++) {
    await act(async () => {
      bridge.message!({
        type: 'dsh://native-notification',
        title: 'Questions',
        body: `Question ${round}/2`,
        tag: `dsh-notification-pending-session-${round}`,
        sessionId: 'session',
        actions: [{ action: 'reply', title: 'Reply', input: true, inputPlaceholder: 'Your answer', inputButtonTitle: 'Submit' }],
      })
    })
    expect(bridge.register).toHaveBeenCalledTimes(round)
    expect(bridge.register.mock.calls[round - 1][0]).toContainEqual({
      id: 'dsh-notification-reply',
      actions: [{ id: 'reply', title: 'Reply', foreground: false, input: true, inputPlaceholder: 'Your answer', inputButtonTitle: 'Submit' }],
    })
    expect(bridge.send).toHaveBeenCalledTimes(round)
    expect(bridge.send.mock.calls[round - 1][0]).toMatchObject({
      body: `Question ${round}/2`,
      actionTypeId: 'dsh-notification-reply',
      extra: { sessionId: 'session', tag: `dsh-notification-pending-session-${round}` },
    })
    expect(bridge.register.mock.invocationCallOrder[round - 1]).toBeLessThan(bridge.send.mock.invocationCallOrder[round - 1])
  }
})
