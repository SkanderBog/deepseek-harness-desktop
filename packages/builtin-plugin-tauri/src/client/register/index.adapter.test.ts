import { expect, it, vi } from 'vitest'
import { defineAdapter } from './index.adapter'

it('keeps current-session and input-action projections over supported core services', () => {
  const binding = {}
  const inputActions = { setDraft: vi.fn() }
  const snapshot = { ids: ['session-1'], byId: {} }
  const offList = vi.fn()
  const offCurrent = vi.fn()
  const list = { getSnapshot: () => snapshot, subscribe: vi.fn((_listener: () => void) => offList) }
  const current = { getSnapshot: () => ({ key: 'session-1' }), subscribe: vi.fn(() => offCurrent) }
  const bindingSource = vi.fn(() => ({ getSnapshot: () => ({ props: { inputActions } }) }))
  const adapter = defineAdapter({
    sessions: { list, binding: () => binding },
    uiSession: { adapter: { current, bindingSource } },
  })

  expect(adapter.sessions.list!.getSnapshot()).toEqual({ ...snapshot, current: 'session-1' })
  expect(adapter.sessions.provideInfo?.('session-1')).toEqual({ props: { inputActions } })
  expect(bindingSource).toHaveBeenCalledWith({ sessionId: 'session-1', binding })
  const listener = vi.fn()
  const dispose = adapter.sessions.list!.subscribe(listener)
  list.subscribe.mock.calls[0][0]()
  expect(listener).toHaveBeenCalledOnce()
  expect(current.subscribe).toHaveBeenCalledWith(listener)
  dispose()
  expect(offList).toHaveBeenCalledOnce()
  expect(offCurrent).toHaveBeenCalledOnce()
})
