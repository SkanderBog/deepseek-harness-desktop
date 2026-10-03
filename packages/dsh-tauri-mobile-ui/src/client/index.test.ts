import { afterEach, expect, it, vi } from 'vitest'
import { apply, inject, name } from './index'
import { registerMobilePreferences } from './register/preferences'
import { registerStyles } from './register/styles'

vi.mock('./register/preferences', () => ({ registerMobilePreferences: vi.fn() }))
vi.mock('./register/styles', () => ({ registerStyles: vi.fn() }))

afterEach(() => vi.restoreAllMocks())

it('assembles mobile-only effects under its own plugin identity', () => {
  const effect = vi.fn()
  apply({ effect } as unknown as Parameters<typeof apply>[0])
  expect(name).toBe('dsh-tauri-mobile-ui')
  expect(inject).toEqual(['slots', 'locale'])
  expect(effect.mock.calls).toEqual([
    [registerStyles, 'dsh-tauri-mobile-ui: styles'],
    [registerMobilePreferences, 'dsh-tauri-mobile-ui: preferences'],
  ])
})
