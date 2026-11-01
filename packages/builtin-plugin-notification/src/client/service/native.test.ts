import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { showNativeNotification } from './native'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => true),
  listen: vi.fn(async () => () => {}),
}))

vi.mock('dsh-tauri/client', () => ({
  getCurrentWindow: () => ({ label: 'window-2' }),
  invoke: mocks.invoke,
  listen: mocks.listen,
}))

beforeEach(() => vi.stubGlobal('location', { origin: 'http://127.0.0.1:3081' }))
afterEach(() => {
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

it('stores the producing window and session in the native activation payload', async () => {
  expect(await showNativeNotification({ title: 'Finished', body: 'Turn complete', tag: 'turn-1', sessionId: 'session-1', deliveryKey: 'session-1:turn:42' })).toBe(true)
  await vi.waitFor(() => {
    expect(mocks.invoke).toHaveBeenCalledWith('plugin:notifications|notify', {
      options: expect.objectContaining({
        title: 'Finished',
        extra: { sessionId: 'session-1', title: 'Finished', tag: 'turn-1', windowLabel: 'window-2', sourceOrigin: 'http://127.0.0.1:3081', deliveryKey: JSON.stringify(['http://127.0.0.1:3081', 'session-1:turn:42']) },
      }),
    })
  })
})
