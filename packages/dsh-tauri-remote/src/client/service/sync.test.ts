import type { MachineRow } from '../types/index'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { store } from '../store/index'
import { answer, replies, resetWire, sent } from '../test-utils/client-mock'
import * as service from './sync'

const baseURL = '/api/tauri/remote'

vi.mock('dsh-tauri/client', async () => (await import('../test-utils/client-mock')).clientMock)

const tick = () => new Promise(resolve => setTimeout(resolve, 0))

const preview = {
  plugins: [{ name: 'p', spec: 'github:a/b', syncable: true }],
  skills: [{ name: 's', root: 'dsh' }],
}

const machineA: MachineRow = {
  id: 'a',
  name: 'alpha',
  host: '10.0.0.1',
  port: 22,
  user: 'root',
  hasPassword: false,
  hasPassphrase: false,
  remotePort: 3080,
}

beforeEach(() => {
  resetWire()
  store.sync.reset()
  store.machines.reset()
  store.machines.commitList({ machines: [machineA], discovered: [], statuses: {} })
})

describe('loadSyncPreview', () => {
  it('publishes the previewed items', async () => {
    answer(() => replies.ok(preview))
    await service.loadSyncPreview()
    expect(store.sync.status).toBe('ready')
    expect(store.sync.preview).toEqual(preview)
    expect(store.sync.error).toBeNull()
  })

  it('reports a preview failure on the panel status line', async () => {
    answer(() => replies.fail('nope'))
    await service.loadSyncPreview()
    expect(store.sync.status).toBe('error')
    expect(store.sync.error).toBe('nope')
  })

  it('reads a malformed preview as a failure', async () => {
    answer(() => replies.ok({ plugins: 'nope' }))
    await service.loadSyncPreview()
    expect(store.sync.error).toBe('malformed sync.preview payload')
  })
})

describe('applySync', () => {
  it('rejects a second submission without clearing the running state', async () => {
    const response = Promise.withResolvers<ReturnType<typeof replies.ok>>()
    let requests = 0
    answer(() => ++requests === 1 ? response.promise : replies.fail('duplicate request'))
    const pending = service.applySync({ machineId: 'a', plugins: [], skills: [] })
    try {
      await expect(service.applySync({ machineId: 'a', plugins: [], skills: [] })).resolves.toMatchObject({ ok: false })
      expect(sent).toHaveLength(1)
      expect(store.sync.applying).toBe(true)
      expect(store.sync.error).toBeNull()
    }
    finally {
      response.resolve(replies.ok({ items: [] }))
      await pending
    }
  })

  it('does not merge results from different target machines', async () => {
    answer(() => replies.ok({ items: [{ kind: 'plugin', name: 'p', ok: false, error: 'failed on a' }] }))
    await service.applySync({ machineId: 'a', plugins: [], skills: [] })
    answer(() => replies.ok({ items: [{ kind: 'skill', name: 's', root: 'dsh', ok: true }] }))
    await service.applySync({ machineId: 'b', plugins: [], skills: [] })
    expect(store.sync.machineId).toBe('b')
    expect(store.sync.results).toEqual([{ kind: 'skill', name: 's', root: 'dsh', ok: true }])
  })

  it('starts fresh results when a same-ID endpoint changes before a new apply', async () => {
    answer(() => replies.ok({ items: [{ kind: 'plugin', name: 'p', ok: false, error: 'old endpoint' }] }))
    await service.applySync({ machineId: 'a', plugins: [], skills: [] })
    store.machines.commitList({ machines: [{ ...machineA, host: '10.0.0.2' }], discovered: [], statuses: {} })
    answer(() => replies.ok({ items: [{ kind: 'skill', name: 's', root: 'dsh', ok: true }] }))
    await service.applySync({ machineId: 'a', plugins: [], skills: [] })
    expect(store.sync.target).toMatchObject({ id: 'a', host: '10.0.0.2' })
    expect(store.sync.results).toEqual([{ kind: 'skill', name: 's', root: 'dsh', ok: true }])
  })

  it('sends the selected items and lands the per-item results', async () => {
    answer(() => replies.ok({ items: [
      { kind: 'plugin', name: 'p', ok: true },
      { kind: 'skill', name: 's', root: 'dsh', ok: false, error: 'exit 1' },
    ] }))
    const result = await service.applySync({ machineId: 'a', plugins: [preview.plugins[0]!], skills: [preview.skills[0]!] })
    expect(result).toEqual({ ok: true })
    expect(sent).toEqual([{ url: `${baseURL}/sync/apply`, http: 'POST', body: {
      machineId: 'a',
      plugins: [{ name: 'p', spec: 'github:a/b' }],
      skills: [{ name: 's', root: 'dsh' }],
    }, params: {} }])
    expect(store.sync.results).toEqual([
      { kind: 'plugin', name: 'p', ok: true },
      { kind: 'skill', name: 's', root: 'dsh', ok: false, error: 'exit 1' },
    ])
    expect(store.sync.applying).toBe(false)
    expect(store.sync.error).toBeNull()
  })

  it('reports an apply failure without dropping the panel', async () => {
    answer(() => replies.fail('ssh down'))
    await expect(service.applySync({ machineId: 'a', plugins: [], skills: [] })).resolves.toEqual({ ok: false, error: 'ssh down' })
    expect(store.sync.applying).toBe(false)
    expect(store.sync.error).toBe('ssh down')
  })

  it('keeps the previous results while a retry is in flight', async () => {
    answer(() => replies.ok({ items: [
      { kind: 'plugin', name: 'p', ok: false, error: 'boom' },
      { kind: 'skill', name: 's', root: 'dsh', ok: true },
    ] }))
    await service.applySync({ machineId: 'a', plugins: [preview.plugins[0]!], skills: [preview.skills[0]!] })

    let release: ((reply: ReturnType<typeof replies.ok>) => void) | undefined
    answer(() => new Promise((resolve) => {
      release = resolve
    }))
    const pending = service.applySync({ machineId: 'a', plugins: [preview.plugins[0]!], skills: [] })
    await tick()
    expect(store.sync.applying).toBe(true)
    expect(store.sync.results).toHaveLength(2)

    release?.(replies.ok({ items: [{ kind: 'plugin', name: 'p', ok: true }] }))
    await pending
    expect(store.sync.results).toEqual([
      { kind: 'plugin', name: 'p', ok: true },
      { kind: 'skill', name: 's', root: 'dsh', ok: true },
    ])
  })

  it('reads a malformed apply payload as a failure', async () => {
    answer(() => replies.ok({}))
    await expect(service.applySync({ machineId: 'a', plugins: [], skills: [] })).resolves.toEqual({ ok: false, error: 'malformed sync.apply payload' })
    expect(store.sync.results).toBeNull()
  })
})
