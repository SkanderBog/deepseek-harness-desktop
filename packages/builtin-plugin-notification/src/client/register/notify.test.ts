import type { LifecycleController } from '../../../../builtin-plugin-tauri/src/client/controller'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { createLifecycleController } from '../../../../builtin-plugin-tauri/src/client/controller'
import { notificationSettings } from '../store'
import { notifyFeature } from './notify'

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: { sessionId?: string, action?: string, sourceOrigin?: string } }) => void>(),
  openSession: vi.fn(() => ({ status: 'opened' })),
  emit: vi.fn(async () => {}),
  show: vi.fn(async () => true),
  readTurnEnd: vi.fn(async () => ({ interrupted: false, turn: 42 })),
  play: vi.fn(),
}))

vi.mock('dsh-tauri/client', () => ({
  defineRegister: (setup: unknown) => setup,
  emit: mocks.emit,
  listen: async (event: string, handler: (event: { payload: { sessionId?: string, action?: string, sourceOrigin?: string } }) => void) => {
    mocks.listeners.set(event, handler)
    return () => mocks.listeners.delete(event)
  },
}))
vi.mock('../locales', () => ({ locale: { text: (key: string) => key } }))
vi.mock('../store', () => ({ notificationSettings: { $state: {} } }))
vi.mock('../service/native', () => ({ showNativeNotification: mocks.show }))
vi.mock('../service/sound', () => ({ createSoundPlayer: () => ({ play: mocks.play, dispose: vi.fn() }) }))
vi.mock('../service/turn-end', () => ({ readTurnEnd: mocks.readTurnEnd }))

beforeEach(() => vi.stubGlobal('location', { origin: 'http://127.0.0.1:3081' }))

afterEach(() => {
  vi.unstubAllGlobals()
  mocks.listeners.clear()
  vi.clearAllMocks()
})

it('opens a session from a historical notification before creating any new notification', async () => {
  const controller = createLifecycleController()
  const setup = notifyFeature as unknown as (controller: LifecycleController, ctx: unknown, adapter: unknown) => void
  setup(controller, { get: () => undefined }, { has: () => false, openSession: mocks.openSession })
  await vi.waitFor(() => expect(mocks.emit).toHaveBeenCalledWith('dsh://notification-ready', { sourceOrigin: 'http://127.0.0.1:3081' }))
  try {
    mocks.emit.mockClear()
    mocks.listeners.get('dsh://notification-ready-request')!({ payload: {} })
    expect(mocks.emit).toHaveBeenCalledWith('dsh://notification-ready', { sourceOrigin: 'http://127.0.0.1:3081' })
    const clicked = mocks.listeners.get('dsh://notification-clicked')!
    clicked({ payload: { sessionId: 'session-before-reload', sourceOrigin: 'http://127.0.0.1:3081' } })
    expect(mocks.openSession).toHaveBeenCalledExactlyOnceWith('session-before-reload')
    clicked({ payload: { sessionId: 'session-before-reload', sourceOrigin: 'http://127.0.0.1:3081', action: 'approve' } })
    clicked({ payload: { sessionId: '' } })
    clicked({ payload: { sessionId: 'wrong-host', sourceOrigin: 'https://other.test' } })
    expect(mocks.openSession).toHaveBeenCalledTimes(1)
  }
  finally {
    controller.dispose()
  }
  expect(mocks.listeners.size).toBe(0)
})

it('gives two guest producers the same turn identity and plays sound only for the accepted delivery', async () => {
  vi.useFakeTimers()
  Object.assign(notificationSettings.$state, { turnComplete: 'always', sound: 'default' })
  const observers = new Set<() => void>()
  let running = true
  const sessions = { list: {
    getSnapshot: () => ({ ids: ['session-1'], byId: { 'session-1': { id: 'session-1', running, updatedAt: 100 } } }),
    subscribe: (fn: () => void) => {
      observers.add(fn)
      return () => observers.delete(fn)
    },
  } }
  const controllers = [createLifecycleController(), createLifecycleController()]
  const setup = notifyFeature as unknown as (controller: LifecycleController, ctx: unknown, adapter: unknown) => void
  mocks.show.mockResolvedValueOnce(true).mockResolvedValueOnce(false)
  try {
    for (const controller of controllers)
      setup(controller, { get: (name: string) => name === 'sessions' ? sessions : undefined }, { has: () => false, sessionList: () => undefined })
    running = false
    observers.forEach(fn => fn())
    await vi.advanceTimersByTimeAsync(250)
    expect(mocks.show).toHaveBeenCalledTimes(2)
    for (const [input] of mocks.show.mock.calls as unknown as [{ deliveryKey: string }][])
      expect(input.deliveryKey).toBe(JSON.stringify(['session-1', 'turn', 42]))
    expect(mocks.play).toHaveBeenCalledOnce()
    expect(mocks.readTurnEnd).toHaveBeenCalledTimes(2)
  }
  finally {
    controllers.forEach(controller => controller.dispose())
    vi.useRealTimers()
  }
})
