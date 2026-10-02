import type { Binding } from '../types'
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve, sep } from 'node:path'
import process from 'node:process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const fixtureParent = fileURLToPath(new URL('../../../../../.temp/', import.meta.url))
const dshHome = vi.hoisted(() => ({ value: '' }))

// 本 worktree 的 node_modules 只有仓库根一层，包内无法解析 lodash-es；优先用
// pnpm 的 hoisted 目录，解析不到（例如完整安装的 CI）时回落到真实模块。
vi.mock('lodash-es', async (importOriginal) => {
  try {
    const { createRequire } = await import('node:module')
    const require = createRequire(new URL('../../../../../node_modules/.pnpm/node_modules/', import.meta.url))
    return await import(pathToFileURL(require.resolve('lodash-es')).href)
  }
  catch {
    return await importOriginal()
  }
})

vi.mock('dsh-tauri', async () => {
  const { defineHostRuntime } = await import('../../../../dsh-tauri/src/host/config/runtime')
  const { defineService } = await import('../../../../dsh-tauri/src/host/service')
  const { fsAtomicDriver } = await import('../../../../dsh-tauri/src/host/utils/driver')
  return {
    defineHostRuntime,
    defineService,
    fsAtomicDriver,
    get DSH_HOME() {
      return dshHome.value
    },
  }
})

let fixture: string
let worktree: typeof import('./worktree')['worktree']
let ledger: typeof import('./ledger')['ledger']

beforeEach(async () => {
  vi.resetModules()
  mkdirSync(fixtureParent, { recursive: true })
  fixture = mkdtempSync(join(fixtureParent, 'worktree-remove-safety-'))
  dshHome.value = join(fixture, 'home')
  mkdirSync(dshHome.value)
  worktree = (await import('./worktree')).worktree
  ledger = (await import('./ledger')).ledger
  const { clearHostRuntime } = await import('../config/runtime')
  clearHostRuntime()
})

afterEach(() => {
  vi.restoreAllMocks()
  const absolute = resolve(fixture)
  const child = relative(realpathSync(fixtureParent), realpathSync(absolute))
  expect(child).not.toBe('')
  expect(child.split(sep)[0]).toMatch(/^worktree-remove-safety-/)
  rmSync(absolute, { recursive: true, force: true })
})

function persistBinding(overrides: Record<string, unknown> = {}): { binding: Binding, path: string, raw: string } {
  const binding = {
    sessionId: 'safety-session',
    sourceSessionId: 'safety-session',
    hash: 'abc123def456',
    dirname: 'repo',
    worktreePath: join(dshHome.value, 'worktrees', 'abc123def456', 'repo'),
    projectPath: '',
    branchName: '(detached)',
    ownsBranch: false,
    createdAt: '2026-07-20T00:00:00.000Z',
    log: [],
    ...overrides,
  } as Binding
  const path = join(dshHome.value, 'ledger', 'safety-session.json')
  const raw = `${JSON.stringify(binding)}\n`
  mkdirSync(join(dshHome.value, 'ledger'), { recursive: true })
  writeFileSync(path, raw)
  expect(ledger.load('safety-session')).toEqual(binding)
  return { binding, path, raw }
}

function markerIn(directory: string): string {
  mkdirSync(directory, { recursive: true })
  const marker = join(directory, 'keep.txt')
  writeFileSync(marker, 'must survive #848\n')
  return marker
}

function linkDirectory(target: string, path: string): void {
  mkdirSync(dirname(path), { recursive: true })
  symlinkSync(target, path, process.platform === 'win32' ? 'junction' : 'dir')
}

describe('worktree deletion safety (#848)', () => {
  it.each(['external', 'home', 'worktrees root', 'hash container', 'wrong hash', 'wrong dirname'])('rejects persisted %s targets and preserves their ledger', async (target) => {
    const directory = {
      'external': join(fixture, 'external-repository'),
      'home': dshHome.value,
      'worktrees root': join(dshHome.value, 'worktrees'),
      'hash container': join(dshHome.value, 'worktrees', 'abc123def456'),
      'wrong hash': join(dshHome.value, 'worktrees', 'def456abc123', 'repo'),
      'wrong dirname': join(dshHome.value, 'worktrees', 'abc123def456', 'other-repo'),
    }[target]!
    const marker = markerIn(directory)
    const persisted = persistBinding({ worktreePath: directory })

    const result = await worktree.remove('safety-session')

    expect(existsSync(marker)).toBe(true)
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it.each(['home', 'worktrees root', 'external', 'other worktree'])('rejects a leaf symlink alias to %s before unlinking dependencies', async (target) => {
    const persisted = persistBinding()
    const directory = {
      'home': dshHome.value,
      'worktrees root': join(dshHome.value, 'worktrees'),
      'external': join(fixture, 'external-repository'),
      'other worktree': join(dshHome.value, 'worktrees', 'def456abc123', 'repo'),
    }[target]!
    const marker = markerIn(directory)
    const dependencies = join(fixture, 'source-dependencies')
    mkdirSync(dependencies)
    linkDirectory(dependencies, join(directory, 'node_modules'))
    linkDirectory(directory, persisted.binding.worktreePath)

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(lstatSync(persisted.binding.worktreePath).isSymbolicLink()).toBe(true)
    expect(lstatSync(join(directory, 'node_modules')).isSymbolicLink()).toBe(true)
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it.each([true, false])('rejects a hash-directory alias with existing target=%s', async (existing) => {
    const persisted = persistBinding()
    const external = join(fixture, 'external-container')
    const marker = markerIn(existing ? join(external, 'repo') : external)
    linkDirectory(external, join(dshHome.value, 'worktrees', 'abc123def456'))

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(lstatSync(join(dshHome.value, 'worktrees', 'abc123def456')).isSymbolicLink()).toBe(true)
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it('rejects a configured worktrees-root alias to DSH_HOME', async () => {
    const persisted = persistBinding()
    const marker = markerIn(join(dshHome.value, 'abc123def456', 'repo'))
    linkDirectory(dshHome.value, join(dshHome.value, 'worktrees'))

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(lstatSync(join(dshHome.value, 'worktrees')).isSymbolicLink()).toBe(true)
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it.each(['root', 'hash', 'leaf'])('rejects a %s trash alias even when the worktree is missing', async (level) => {
    const persisted = persistBinding()
    const external = join(fixture, 'external-trash-target')
    const marker = markerIn(external)
    const components = level === 'root' ? [] : level === 'hash' ? ['abc123def456'] : ['abc123def456', 'repo']
    const alias = join(dshHome.value, '.trash', ...components)
    linkDirectory(external, alias)

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(lstatSync(alias).isSymbolicLink()).toBe(true)
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it.each([
    { hash: '..', dirname: '..' },
    { hash: 'abc123def456', dirname: '../..' },
    { hash: 'abc123def456', dirname: '..\\..' },
    { hash: null },
    { dirname: ['repo'] },
    { worktreePath: null },
    { worktreePath: 848 },
  ])('rejects malformed persisted identity %j without cleaning the binding', async (overrides) => {
    const marker = markerIn(dshHome.value)
    const persisted = persistBinding(overrides)

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
  })

  it('rejects an external alias even when it resolves to the expected worktree', async () => {
    const persisted = persistBinding()
    const marker = markerIn(persisted.binding.worktreePath)
    const alias = join(fixture, 'worktree-alias')
    linkDirectory(persisted.binding.worktreePath, alias)
    const aliased = persistBinding({ worktreePath: alias })

    const result = await worktree.remove('safety-session')

    expect(result).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(lstatSync(alias).isSymbolicLink()).toBe(true)
    expect(readFileSync(aliased.path, 'utf8')).toBe(aliased.raw)
  })

  it('keeps an invalid missing-path binding during recovery instead of enabling orphan fallback', async () => {
    const expected = join(dshHome.value, 'worktrees', 'abc123def456', 'repo')
    const marker = markerIn(expected)
    const persisted = persistBinding({ worktreePath: join(fixture, 'missing-external') })

    const recovered = await worktree.recover()

    expect(recovered).toEqual({ ok: true, resumed: 0, swept: 0, pruned: 0 })
    expect(existsSync(persisted.path)).toBe(true)
    expect(readFileSync(persisted.path, 'utf8')).toBe(persisted.raw)
    expect(await worktree.remove('safety-session', 'abc123def456/repo')).toMatchObject({ ok: false })
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
  })

  it('does not sweep external children through a configured trash-root alias', async () => {
    const external = join(fixture, 'external-trash-root')
    const marker = markerIn(join(external, 'abc123def456', 'repo'))
    linkDirectory(external, join(dshHome.value, '.trash'))

    const recovered = await worktree.recover()

    expect(existsSync(marker)).toBe(true)
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
    expect(recovered).toEqual({ ok: true, resumed: 0, swept: 0, pruned: 0 })
    expect(lstatSync(join(dshHome.value, '.trash')).isSymbolicLink()).toBe(true)
  })

  it('removes only the normal bound worktree and its stale trash', async () => {
    const persisted = persistBinding()
    markerIn(persisted.binding.worktreePath)
    const stale = join(dshHome.value, '.trash', 'abc123def456', 'repo')
    markerIn(stale)
    const sibling = markerIn(join(dshHome.value, 'worktrees', 'def456abc123', 'repo'))
    const external = markerIn(join(fixture, 'external-repository'))

    const result = await worktree.remove('safety-session')

    expect(result).toEqual({ ok: true, worktreePath: persisted.binding.worktreePath })
    expect(existsSync(persisted.binding.worktreePath)).toBe(false)
    expect(existsSync(stale)).toBe(false)
    expect(existsSync(persisted.path)).toBe(false)
    expect(readFileSync(sibling, 'utf8')).toBe('must survive #848\n')
    expect(readFileSync(external, 'utf8')).toBe('must survive #848\n')
  })

  it('completes an already missing valid target without deleting unrelated data', async () => {
    const persisted = persistBinding()
    const marker = markerIn(dshHome.value)

    const result = await worktree.remove('safety-session')

    expect(result).toEqual({ ok: true, worktreePath: persisted.binding.worktreePath })
    expect(existsSync(persisted.path)).toBe(false)
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
  })

  it('supports a safe legacy orphan key with no binding', async () => {
    const path = join(dshHome.value, 'worktrees', 'legacy-hash', 'repo')
    markerIn(path)
    const marker = markerIn(dshHome.value)

    const result = await worktree.remove('unbound-session', 'legacy-hash/repo')

    expect(result).toEqual({ ok: true, worktreePath: path.replaceAll('\\', '/') })
    expect(existsSync(path)).toBe(false)
    expect(readFileSync(marker, 'utf8')).toBe('must survive #848\n')
  })
})
