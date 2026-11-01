// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { useListen } from './use-listen'

const mocks = vi.hoisted(() => ({
  unlisten: vi.fn(),
  listen: vi.fn(),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.restoreAllMocks()
})

it('scopes DSH events to the current window and preserves explicit targets and ordinary events', async () => {
  mocks.listen.mockResolvedValue(mocks.unlisten)
  const explicit = { target: { kind: 'Any' as const } }
  const hook = renderHook(() => {
    useListen('dsh://window:close', vi.fn())
    useListen('dsh://explicit', vi.fn(), explicit)
    useListen('macos-menu-rebuilt', vi.fn())
  })
  expect(mocks.listen).toHaveBeenCalledWith('dsh://window:close', expect.any(Function), { target: { kind: 'Window', label: 'main' } })
  expect(mocks.listen).toHaveBeenCalledWith('dsh://explicit', expect.any(Function), explicit)
  expect(mocks.listen).toHaveBeenCalledWith('macos-menu-rebuilt', expect.any(Function), undefined)
  await act(async () => {})
  hook.unmount()
  expect(mocks.unlisten).toHaveBeenCalledTimes(3)
})
