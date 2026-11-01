import type { HostContext, IndexInjectRow } from './types'
import { runInNewContext } from 'node:vm'
import { afterEach, expect, it, vi } from 'vitest'
import { apply } from './apply'

vi.mock('./server', () => ({ server: () => {} }))
vi.mock('./service/gate', () => ({ gate: { attach: () => () => {} } }))

afterEach(() => {
  vi.unstubAllEnvs()
})

function injections(embedded: string): IndexInjectRow[] {
  vi.stubEnv('DSH_TAURI_EMBEDDED', embedded)
  const rows: IndexInjectRow[] = []
  const ctx = {
    effect(callback: () => void) { callback() },
    on(event: string, listener: (table: IndexInjectRow[]) => void) {
      expect(event).toBe('webserver/index-inject')
      listener(rows)
      return () => {}
    },
  } as unknown as HostContext
  apply(ctx)
  return rows
}

it('embedded host marker preserves the native desktop Browser bridge', () => {
  const browser = { acquire() {} }
  const native = { protocolVersion: 1, browser }
  const page = { dshDesktop: native, location: { search: '?dshDesktop' }, URLSearchParams, navigator: { userAgent: 'desktop regression' } }
  const rows = injections('1')
  expect(rows).toHaveLength(2)
  for (const row of rows) {
    if (row.kind === 'script')
      runInNewContext(row.text, page)
  }
  expect(page.dshDesktop).toBe(native)
  expect(page.dshDesktop.browser).toBe(browser)
})

it.each(['?dshDesktop', ''])('embedded host marker installs fallback only for marked pages: %s', async (search) => {
  const page: { location: { search: string }, navigator: { userAgent: string }, dshDesktop?: { protocolVersion: number, deviceInfo: () => Promise<string> } } = { location: { search }, navigator: { userAgent: 'desktop regression' } }
  const rows = injections('1')
  expect(rows).toHaveLength(2)
  for (const row of rows) {
    if (row.kind === 'script')
      runInNewContext(row.text, { ...page, URLSearchParams, globalThis: page })
  }
  if (search === '') {
    expect(page.dshDesktop).toBeUndefined()
    return
  }
  expect(page.dshDesktop?.protocolVersion).toBe(1)
  expect(await page.dshDesktop?.deviceInfo()).toBe('desktop regression')
})

it('embedded host injects the NearToken affiliate code as a page global', () => {
  expect(injections('1').filter(row => row.kind === 'global')).toEqual([
    { kind: 'global', name: 'NEARTOKEN_AFF', value: 'yKfg' },
  ])
})

it('non-embedded host leaves the index injection table unchanged', () => {
  expect(injections('0')).toEqual([])
})
