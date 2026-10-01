// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { defineStore } from 'valtio-define'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { APPEARANCE_DEFAULTS } from '../../../packages/dsh-tauri/src/shared/appearance'
import { ConfigAppearance } from './appearance'

const mocks = vi.hoisted(() => ({ setting: {} as any, toast: vi.fn() }))
vi.mock('@/store', () => ({ store: { get setting() {
  return mocks.setting
} } }))
vi.mock('@/utils/toast', () => ({ toast: mocks.toast }))
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function setup(fail = false) {
  const update = vi.fn(async (value) => {
    if (fail)
      throw new Error('disk write failed')
    mocks.setting.$patch(value)
  })
  mocks.setting = defineStore({
    state: () => ({ appearance: { palette: 'nord', terminal: false, opacity: 70 } }),
    actions: { update },
  })
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
  render(<QueryClientProvider client={client}><ConfigAppearance /></QueryClientProvider>)
  return update
}

describe('appearance settings controls', () => {
  it('provides an operable terminal switch and resets all preferences', async () => {
    const update = setup()
    const toggle = screen.getByRole('switch', { name: 'appearance.terminal' })
    fireEvent.click(toggle)
    await waitFor(() => expect(update).toHaveBeenCalledWith({ appearance: { palette: 'nord', terminal: true, opacity: 70 } }))
    await waitFor(() => expect((toggle as HTMLInputElement).checked).toBe(true))
    expect(screen.getByRole('status').textContent).toBe('appearance.restart')
    const reset = screen.getByRole('button', { name: 'appearance.reset' }) as HTMLButtonElement
    await waitFor(() => expect(reset.disabled).toBe(false))
    fireEvent.click(reset)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ appearance: APPEARANCE_DEFAULTS }))
    await waitFor(() => expect(screen.queryByRole('status')).toBeNull())
    expect((toggle as HTMLInputElement).checked).toBe(false)
  })

  it('reports a failed save and keeps the previously saved preference', async () => {
    setup(true)
    const toggle = screen.getByRole('switch', { name: 'appearance.terminal' })
    fireEvent.click(toggle)
    await waitFor(() => expect(mocks.toast).toHaveBeenCalledWith('appearance.save_failed', { variant: 'danger' }))
    expect((toggle as HTMLInputElement).checked).toBe(false)
    expect(mocks.setting.appearance.terminal).toBe(false)
  })
})
