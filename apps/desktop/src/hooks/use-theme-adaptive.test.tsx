// @vitest-environment jsdom
import { cleanup, renderHook } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { resolveTheme, useThemeAdaptive } from './use-theme-adaptive'

const mocks = vi.hoisted(() => ({
  preference: null as 'dark' | 'light' | 'system' | null,
  systemDark: false,
  dshStyle: {} as { colorScheme?: 'dark' | 'light' },
}))
vi.mock('./use-dsh-style', () => ({ useDshStyle: () => [mocks.dshStyle] }))
vi.mock('./use-invoke', () => ({ useInvoke: (command: string) => {
  expect(command).toBe('get_dsh_theme')
  return mocks.preference
} }))
vi.mock('@reause/core', async importOriginal => ({
  ...await importOriginal<typeof import('@reause/core')>(),
  usePreferredDark: () => mocks.systemDark,
}))

afterEach(() => {
  cleanup()
  delete document.documentElement.dataset.theme
  mocks.dshStyle = {}
  vi.restoreAllMocks()
})

it.each([
  ['dark', false, 'dark'],
  ['light', true, 'light'],
  ['system', false, 'light'],
  ['system', true, 'dark'],
  [null, false, 'light'],
  [undefined, true, 'dark'],
] as const)('resolves %s with system dark=%s to %s', (preference, systemDark, theme) => {
  expect(resolveTheme(preference, systemDark)).toBe(theme)
})

it('uses preference and system changes until the embedded document reports its color scheme', () => {
  mocks.preference = 'light'
  mocks.systemDark = true
  const hook = renderHook(() => useThemeAdaptive())
  expect(document.documentElement.dataset.theme).toBe('light')
  mocks.preference = 'system'
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('dark')
  mocks.systemDark = false
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('light')
  mocks.preference = 'dark'
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('dark')
  mocks.dshStyle = { colorScheme: 'light' }
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('light')
  mocks.dshStyle = { colorScheme: 'dark' }
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('dark')
  mocks.preference = 'light'
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('dark')
  mocks.dshStyle = {}
  hook.rerender()
  expect(document.documentElement.dataset.theme).toBe('light')
})
