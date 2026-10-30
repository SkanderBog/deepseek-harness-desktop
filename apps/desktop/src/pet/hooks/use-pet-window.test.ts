// @vitest-environment jsdom
import { cleanup, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { usePetWindowSize } from './use-pet-window'

const setSizeMock = vi.hoisted(() => vi.fn<(size: unknown) => Promise<void>>())

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve()),
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setSize: setSizeMock }),
  PhysicalSize: class {
    constructor(public width: number, public height: number) {}
  },
}))

describe('usePetWindowSize', () => {
  const originalRatio = globalThis.devicePixelRatio

  beforeEach(() => {
    setSizeMock.mockReset()
    setSizeMock.mockResolvedValue()
  })

  afterEach(() => {
    cleanup()
    globalThis.devicePixelRatio = originalRatio
  })

  it('keeps the CSS viewport at the intended size under a 1.5 device pixel ratio', () => {
    globalThis.devicePixelRatio = 1.5
    renderHook(() => usePetWindowSize(352, 9 / 16, true))
    expect(setSizeMock).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ width: 630, height: 420 }))
  })

  it('matches logical sizing when the ratio is 1', () => {
    globalThis.devicePixelRatio = 1
    renderHook(() => usePetWindowSize(220, 9 / 16, true))
    expect(setSizeMock).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ width: 420, height: 206 }))
  })

  it('does not resize a hidden window', () => {
    renderHook(() => usePetWindowSize(220, 9 / 16, false))
    expect(setSizeMock).not.toHaveBeenCalled()
  })
})
