// @vitest-environment jsdom
import type { RemoteKey } from '../locales/index'
import type { MachineRow } from '../types/index'
import { fireEvent, screen, waitFor } from '@testing-library/dom'
import { cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { en } from '../locales/index'
import { applySync } from '../service/sync'
import { store } from '../store/index'
import { answer, replies, resetWire, sent } from '../test-utils/client-mock'
import { SyncPanel } from './sync-panel'

const baseURL = '/api/tauri/remote'

vi.mock('dsh-tauri-ui/client', async () => (await import('../test-utils/ui-mock')).uiMock)

vi.mock('dsh-tauri/client', async () => (await import('../test-utils/client-mock')).clientMock)

beforeEach(() => {
  resetWire()
  store.machines.reset()
  store.sync.reset()
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const t = ((key: string) => (en as Record<string, string>)[key] ?? key) as (key: RemoteKey) => string

const regexOf = (text: string): RegExp => new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))

function routeWire(routes: Record<string, unknown>, failures: string[] = []): void {
  answer(call => failures.includes(call.url)
    ? replies.fail(`${call.url} broke`)
    : replies.ok(routes[call.url] ?? {}))
}

const preview = {
  plugins: [
    { name: 'dsh-market', spec: 'github:omdsh/dsh-market', syncable: true },
    { name: 'local-thing', spec: 'link:../local', syncable: false, reason: 'local-path dependency; it cannot be resolved on the remote' },
  ],
  skills: [
    { name: 'alpha', root: 'dsh' },
    { name: 'beta', root: 'agents' },
  ],
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

const connectedStatus = { state: 'connected' as const, tunnelBaseUrl: 'http://127.0.0.1:1' }

function seedConnected(): void {
  store.machines.commitList({ machines: [machineA], discovered: [], statuses: { a: connectedStatus } })
  store.sync.commitPreview(preview)
}

describe('syncPanel', () => {
  it('starts with every syncable item ticked and keeps independent tick state', async () => {
    routeWire({ [`${baseURL}/machines`]: { items: [] } })
    seedConnected()
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-row-dsh-market')).toBeTruthy())
    expect(screen.getByTestId('sync-row-dsh-market').getAttribute('aria-checked')).toBe('true')
    expect(screen.getByTestId('sync-row-alpha').getAttribute('aria-checked')).toBe('true')
    expect(screen.getByTestId('sync-row-beta').getAttribute('aria-checked')).toBe('true')
    expect(screen.getByTestId('sync-row-local-thing').getAttribute('aria-checked')).toBe('false')
    expect(screen.getByTestId('sync-selected').textContent).toContain('1 plugins and 2 skills selected')

    fireEvent.click(screen.getByTestId('sync-row-dsh-market'))
    expect(screen.getByTestId('sync-row-dsh-market').getAttribute('aria-checked')).toBe('false')
    expect(screen.getByTestId('sync-row-alpha').getAttribute('aria-checked')).toBe('true')
    expect(screen.getByTestId('sync-selected').textContent).toContain('0 plugins and 2 skills selected')

    fireEvent.click(screen.getByText('Clear'))
    expect(screen.getByTestId('sync-row-alpha').getAttribute('aria-checked')).toBe('false')
    fireEvent.click(screen.getByText('Select all'))
    expect(screen.getByTestId('sync-row-dsh-market').getAttribute('aria-checked')).toBe('true')
  })

  it('disables unsyncable plugins and shows their reason', async () => {
    routeWire({ [`${baseURL}/machines`]: { items: [] } })
    seedConnected()
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-row-local-thing')).toBeTruthy())
    expect(screen.getByTestId('sync-row-local-thing').hasAttribute('disabled')).toBe(true)
    expect(screen.getByText(/local-path dependency/)).toBeTruthy()
  })

  it('sends the whole ticked selection to sync.apply', async () => {
    routeWire({ [`${baseURL}/sync/apply`]: { items: [] } })
    seedConnected()
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-apply')).toBeTruthy())
    fireEvent.click(screen.getByTestId('sync-apply'))
    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    expect(applyCalls()[0]).toEqual({
      machineId: 'a',
      plugins: [{ name: 'dsh-market', spec: 'github:omdsh/dsh-market' }],
      skills: [{ name: 'alpha', root: 'dsh' }, { name: 'beta', root: 'agents' }],
    })
  })

  it('shows per-item live progress while an apply runs', async () => {
    routeWire({ [`${baseURL}/machines`]: { items: [] } })
    seedConnected()
    store.sync.beginApply()
    store.machines.commitList({
      machines: [machineA],
      discovered: [],
      statuses: { a: { ...connectedStatus, progress: { phase: 'syncing', attempt: 3, total: 5, item: 'dsh-tauri-pet' } } },
    })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-progress')).toBeTruthy())
    expect(screen.getByTestId('sync-progress').textContent).toContain('3/5')
    expect(screen.getByTestId('sync-progress').textContent).toContain('dsh-tauri-pet')
    const fill = screen.getByTestId('sync-progress').querySelector('[class~="bg-brand"]') as HTMLElement
    expect(fill.style.width).toBe('40%')
    expect(screen.getByTestId('sync-apply').textContent).toContain('Syncing')
  })

  it('restores the running target after remount and blocks a second submission', async () => {
    routeWire({})
    seedConnected()
    store.sync.beginApply('a', machineA)
    store.machines.commitList({
      machines: [machineA],
      discovered: [],
      statuses: { a: { ...connectedStatus, progress: { phase: 'syncing', attempt: 0, total: 3 } } },
    })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-progress')).toBeTruthy())
    expect(screen.getByTestId('sync-progress').textContent).toContain('Connecting and preparing sync')
    expect(screen.getByTestId('sync-apply').hasAttribute('disabled')).toBe(true)
    fireEvent.click(screen.getByTestId('sync-apply'))
    expect(applyCalls()).toHaveLength(0)
  })

  it('blocks duplicate submission using host progress after client state resets', async () => {
    routeWire({})
    seedConnected()
    store.machines.commitList({
      machines: [machineA],
      discovered: [],
      statuses: { a: { ...connectedStatus, progress: { phase: 'syncing', attempt: 0, total: 1 } } },
    })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-apply').hasAttribute('disabled')).toBe(true))
    expect(screen.getByTestId('sync-progress').textContent).toContain('Connecting and preparing sync')
    fireEvent.click(screen.getByTestId('sync-apply'))
    expect(applyCalls()).toHaveLength(0)
  })

  it('labels live progress with the running machine when another target is selected', async () => {
    routeWire({})
    const other = { ...machineA, id: 'b', name: 'beta' }
    store.machines.commitList({ machines: [machineA, other], discovered: [], statuses: { a: connectedStatus, b: connectedStatus } })
    store.sync.commitPreview(preview)
    const view = render(<SyncPanel t={t} />)
    fireEvent.click(screen.getByTestId('sync-target-a'))
    store.machines.commitList({
      machines: [machineA, other],
      discovered: [],
      statuses: { a: connectedStatus, b: { ...connectedStatus, progress: { phase: 'syncing', attempt: 2, total: 3, item: 'plugin-b' } } },
    })
    view.rerender(<SyncPanel t={t} />)
    expect(screen.getByTestId('sync-target-a').getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByTestId('sync-progress').textContent).toContain('beta')
    expect(screen.getByTestId('sync-progress').textContent).toContain('2/3')
    expect(screen.getByTestId('sync-progress').textContent).toContain('plugin-b')
  })

  it.each([
    { name: 'renamed' },
    { host: '10.0.0.2' },
    { port: 2222 },
    { user: 'deploy' },
    { profileName: 'work' },
  ])('blocks stale retries after a same-ID target edit %j', async (edit) => {
    routeWire({ [`${baseURL}/sync/apply`]: { items: [{ kind: 'skill', name: 'alpha', root: 'dsh', ok: false, error: 'old failure' }] } })
    seedConnected()
    await applySync({ machineId: 'a', plugins: [], skills: preview.skills })
    const view = render(<SyncPanel t={t} />)
    expect(screen.getByTestId('sync-retry').hasAttribute('disabled')).toBe(false)
    store.machines.commitList({ machines: [{ ...machineA, ...edit }], discovered: [], statuses: { a: connectedStatus } })
    view.rerender(<SyncPanel t={t} />)
    expect(screen.getByTestId('sync-retry').hasAttribute('disabled')).toBe(true)
    expect(screen.getByTestId('sync-results').textContent).toContain('Sync results for alpha')
    fireEvent.click(screen.getByTestId('sync-retry'))
    expect(applyCalls()).toHaveLength(1)
  })

  it('keeps the applied name when a target is renamed during a sync', async () => {
    const response = Promise.withResolvers<ReturnType<typeof replies.ok>>()
    answer(() => response.promise)
    seedConnected()
    const pending = applySync({ machineId: 'a', plugins: [], skills: preview.skills })
    store.machines.commitList({ machines: [{ ...machineA, name: 'renamed' }], discovered: [], statuses: { a: connectedStatus } })
    response.resolve(replies.ok({ items: [{ kind: 'skill', name: 'alpha', root: 'dsh', ok: false }] }))
    await pending
    render(<SyncPanel t={t} />)
    expect(screen.getByTestId('sync-results').textContent).toContain('Sync results for alpha')
    expect(screen.getByTestId('sync-retry').hasAttribute('disabled')).toBe(true)
  })

  it('labels results by machine and prevents retry on another target', async () => {
    routeWire({})
    const other = { ...machineA, id: 'b', name: 'beta' }
    store.machines.commitList({
      machines: [machineA, other],
      discovered: [],
      statuses: { a: connectedStatus, b: connectedStatus },
    })
    store.sync.commitPreview(preview)
    store.sync.beginApply('a', machineA)
    store.sync.endApply()
    store.sync.mergeResults([{ kind: 'skill', name: 'alpha', root: 'dsh', ok: false, error: 'missing' }])
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-retry')).toBeTruthy())
    fireEvent.click(screen.getByTestId('sync-target-b'))
    expect(screen.getByTestId('sync-results').textContent).toContain('Sync results for alpha')
    expect(screen.getByTestId('sync-retry').hasAttribute('disabled')).toBe(true)
    fireEvent.click(screen.getByTestId('sync-retry'))
    expect(applyCalls()).toHaveLength(0)
    fireEvent.click(screen.getByTestId('sync-target-a'))
    expect(screen.getByTestId('sync-retry').hasAttribute('disabled')).toBe(false)
  })

  it.each(['remembered', 'selected'])('falls back after the %s target disconnects', async (source) => {
    routeWire({ [`${baseURL}/sync/apply`]: { items: [] } })
    const other = { ...machineA, id: 'b', name: 'beta' }
    store.machines.commitList({
      machines: [machineA, other],
      discovered: [],
      statuses: { a: connectedStatus, b: connectedStatus },
    })
    store.sync.commitPreview(preview)
    if (source === 'remembered') {
      store.sync.beginApply('b', other)
      store.sync.endApply()
    }
    const view = render(<SyncPanel t={t} />)
    if (source === 'selected')
      fireEvent.click(screen.getByTestId('sync-target-b'))
    store.machines.commitList({
      machines: [machineA, other],
      discovered: [],
      statuses: { a: connectedStatus, b: { state: 'disconnected' } },
    })
    view.rerender(<SyncPanel t={t} />)
    expect(screen.getByTestId('sync-target-a').getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(screen.getByTestId('sync-apply'))
    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    expect(applyCalls()[0]?.machineId).toBe('a')
  })

  it('blocks target switching while another sync is active', async () => {
    routeWire({})
    const other = { ...machineA, id: 'b', name: 'beta' }
    store.machines.commitList({
      machines: [machineA, other],
      discovered: [],
      statuses: { a: connectedStatus, b: connectedStatus },
    })
    store.sync.commitPreview(preview)
    store.sync.beginApply('b', other)
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-target-a')).toBeTruthy())
    fireEvent.click(screen.getByTestId('sync-target-a'))
    expect(screen.getByTestId('sync-apply').hasAttribute('disabled')).toBe(true)
    expect(screen.getByTestId('sync-target-b').getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByTestId('sync-apply').textContent).toContain('Syncing')
  })

  it('renders partial failures per item with reasons and a retry that re-sends only them', async () => {
    routeWire({ [`${baseURL}/sync/apply`]: { items: [] }, [`${baseURL}/machines`]: { items: [] } })
    seedConnected()
    store.sync.beginApply('a', machineA)
    store.sync.endApply()
    store.sync.mergeResults([
      { kind: 'plugin', name: 'dsh-market', ok: true },
      { kind: 'skill', name: 'alpha', root: 'dsh', ok: false, error: 'exit 1: read-only file system' },
    ])
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-results')).toBeTruthy())
    expect(screen.getByTestId('sync-result-dsh-market').dataset.ok).toBe('true')
    const failed = screen.getByTestId('sync-result-alpha')
    expect(failed.dataset.ok).toBe('false')
    expect(failed.textContent).toContain('read-only file system')
    expect(failed.textContent).toContain('(dsh)')
    expect(screen.getByText(/1 succeeded, 1 failed/)).toBeTruthy()

    fireEvent.click(screen.getByTestId('sync-retry'))
    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    expect(applyCalls()[0]).toEqual({
      machineId: 'a',
      plugins: [],
      skills: [{ name: 'alpha', root: 'dsh' }],
    })
  })

  it('keeps the failure headline cause-first and shows the raw output on demand', async () => {
    routeWire({})
    seedConnected()
    store.sync.mergeResults([{
      kind: 'plugin',
      name: 'dsh-better-sidebar',
      ok: false,
      error: 'exit 1: make: *** [pty.target.mk:119] Error 127 | gyp ERR! stack Error: `make` failed with exit code: 2',
      log: 'Progress: resolved 584\nmake: *** [pty.target.mk:119] Error 127\n[ERR_PNPM_PREPARE_PACKAGE] failed',
    }])
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-results')).toBeTruthy())
    const row = screen.getByTestId('sync-result-dsh-better-sidebar')
    expect(row.textContent).toContain('Error 127')
    expect(row.textContent).not.toContain('Progress: resolved')
    const toggle = screen.getByTestId('sync-log-toggle-dsh-better-sidebar')
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    expect(screen.queryByTestId('sync-log')).toBeNull()
    fireEvent.click(toggle)
    expect(screen.getByTestId('sync-log-toggle-dsh-better-sidebar').getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByTestId('sync-log').textContent).toContain('ERR_PNPM_PREPARE_PACKAGE')
  })

  it('renders a complete failure with every reason visible', async () => {
    routeWire({})
    seedConnected()
    store.sync.mergeResults([
      { kind: 'plugin', name: 'dsh-market', ok: false, error: 'exit 1: ERR_PNPM_NO_MATCH' },
      { kind: 'skill', name: 'alpha', root: 'dsh', ok: false, error: 'exit 1: read-only file system' },
    ])
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-results')).toBeTruthy())
    expect(screen.getByText(/0 succeeded, 2 failed/)).toBeTruthy()
    expect(screen.getByTestId('sync-result-dsh-market').textContent).toContain('ERR_PNPM_NO_MATCH')
    expect(screen.getByTestId('sync-result-alpha').textContent).toContain('read-only file system')
  })

  it('invalidates stale retry results after an unconfirmed apply even when the preview refreshes', async () => {
    routeWire({ [`${baseURL}/sync/preview`]: preview }, [`${baseURL}/sync/apply`])
    seedConnected()
    store.sync.beginApply('a', machineA)
    store.sync.endApply()
    store.sync.mergeResults([{ kind: 'plugin', name: 'dsh-market', ok: false, error: 'earlier failure' }])
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-apply')).toBeTruthy())
    fireEvent.click(screen.getByTestId('sync-apply'))
    await waitFor(() => expect(screen.getByText(regexOf(`Sync completion is unconfirmed; check the remote before retrying: ${baseURL}/sync/apply broke`))).toBeTruthy())
    fireEvent.click(screen.getByText('Refresh list'))
    await waitFor(() => expect(apiMethods()).toContain(`${baseURL}/sync/preview`))
    await waitFor(() => expect(store.sync.status).toBe('ready'))
    expect(screen.queryByTestId('sync-result-dsh-market')).toBeNull()
    expect(screen.queryByTestId('sync-retry')).toBeNull()
    expect(applyCalls()).toHaveLength(1)
  })

  it('disables apply with an empty tick set', async () => {
    routeWire({ [`${baseURL}/machines`]: { items: [] } })
    seedConnected()
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByText('Clear')).toBeTruthy())
    fireEvent.click(screen.getByText('Clear'))
    expect(screen.getByTestId('sync-apply').hasAttribute('disabled')).toBe(true)
  })

  it('loads the machine list itself and offers the connected machine as the target', async () => {
    routeWire({
      [`${baseURL}/machines`]: { items: [{ ...machineA, state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:1' }] },
      [`${baseURL}/sync/preview`]: preview,
    })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-target-a')).toBeTruthy())
    expect(apiMethods()).toContain(`${baseURL}/machines`)
    expect(screen.getByTestId('sync-apply').textContent).toContain('Sync to alpha')
    expect(screen.queryByTestId('sync-empty')).toBeNull()
  })

  it('points a remote-session instance back at the initiating machine', async () => {
    routeWire({ [`${baseURL}/machines`]: { items: [] }, [`${baseURL}/session/role`]: { role: 'remote', remote: true, origin: 'ops' } })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-remote-note')).toBeTruthy())
    expect(screen.getByTestId('sync-remote-note').textContent).toContain('Sync to remote')
    expect(screen.queryByTestId('sync-apply')).toBeNull()
  })

  it('shows the not-connected note when no machine is connected', async () => {
    routeWire({})
    store.machines.commitList({ machines: [machineA], discovered: [], statuses: { a: { state: 'disconnected' } } })
    store.sync.commitPreview(preview)
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByTestId('sync-empty')).toBeTruthy())
    expect(screen.getByTestId('sync-empty').textContent).toContain('No connected machines')
    expect(screen.queryByTestId('sync-apply')).toBeNull()
  })

  it('shows the preview load failure with the reason', async () => {
    routeWire({}, [`${baseURL}/sync/preview`])
    store.machines.commitList({ machines: [], discovered: [], statuses: {} })
    render(<SyncPanel t={t} />)
    await waitFor(() => expect(screen.getByText(regexOf(`Failed to load the sync list: ${baseURL}/sync/preview broke`))).toBeTruthy())
  })
})

function apiMethods(): string[] {
  return sent.map(call => call.url)
}

function applyCalls(): Array<Record<string, unknown>> {
  return sent.filter(call => call.url === `${baseURL}/sync/apply`).map(call => call.body)
}
