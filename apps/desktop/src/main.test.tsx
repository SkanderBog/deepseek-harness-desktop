// @vitest-environment jsdom
import type { ReactNode } from 'react'
import { cleanup, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn(() => new Promise(() => {})),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isMaximized: () => new Promise(() => {}),
    isFullscreen: () => new Promise(() => {}),
  }),
}))
vi.mock('@overlastic/react', () => ({ OverlaysProvider: ({ children }: { children: ReactNode }) => children }))
vi.mock('@tanstack/react-query', () => ({ QueryClientProvider: ({ children }: { children: ReactNode }) => children }))
vi.mock('./components/toast-provider', () => ({ ToastProvider: ({ children }: { children: ReactNode }) => children }))
vi.mock('./config/client', () => ({ queryClient: {} }))
vi.mock('./hooks/use-theme-adaptive', () => ({
  resolveTheme: (_preference: unknown, dark: boolean) => dark ? 'dark' : 'light',
}))
vi.mock('./layout', () => ({ App: () => <div>shell mounted</div> }))
vi.mock('@/utils/logger', () => ({}))

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

it('mounts visible shell content while optional native appearance calls remain pending', async () => {
  document.body.innerHTML = '<div id=\"root\"></div>'
  Object.assign(window, {
    __TAURI_INTERNALS__: {},
    __DSH_FRAME__: true,
    matchMedia: () => ({ matches: false }),
  })

  await import('./main')

  expect(await screen.findByText('shell mounted')).toBeTruthy()
  expect(invoke).not.toHaveBeenCalled()
})
