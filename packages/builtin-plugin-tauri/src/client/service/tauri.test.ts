import { afterEach, expect, it, vi } from 'vitest'
import { emit, listen, once } from './tauri'

afterEach(() => vi.unstubAllGlobals())

it('skips native event IPC in a browser without a desktop bridge', async () => {
  vi.stubGlobal('window', {})
  vi.stubGlobal('isTauri', false)
  await expect(emit('dsh://style', {})).resolves.toBeUndefined()
  const handler = vi.fn()
  const off = await listen('dsh://appearance', handler)
  const offOnce = await once('session:update', handler)
  expect(off).toBeTypeOf('function')
  expect(offOnce).toBeTypeOf('function')
  off()
  offOnce()
  expect(handler).not.toHaveBeenCalled()
})

it('keeps desktop event scope and explicit targets when native IPC is available', async () => {
  const invoke = vi.fn(async () => 7)
  vi.stubGlobal('isTauri', true)
  vi.stubGlobal('window', {
    __TAURI_INTERNALS__: { invoke, transformCallback: () => 42, metadata: { currentWindow: { label: 'harness-1' } } },
  })
  const handler = vi.fn()
  await emit('dsh://style', { color: 'red' })
  await listen('dsh://appearance', handler)
  await once('dsh://appearance', handler, { target: { kind: 'Window', label: 'other' } })
  await emit('session:update', {})
  expect(invoke.mock.calls).toEqual([
    ['plugin:event|emit_to', { target: { kind: 'Window', label: 'harness-1' }, event: 'dsh://style', payload: { color: 'red' } }, undefined],
    ['plugin:event|listen', { event: 'dsh://appearance', target: { kind: 'Window', label: 'harness-1' }, handler: 42 }, undefined],
    ['plugin:event|listen', { event: 'dsh://appearance', target: { kind: 'Window', label: 'other' }, handler: 42 }, undefined],
    ['plugin:event|emit', { event: 'session:update', payload: {} }, undefined],
  ])
})
