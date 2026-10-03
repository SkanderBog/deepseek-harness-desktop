// @vitest-environment jsdom
import type { ReactNode } from 'react'
import type { ModeSelectProps } from './mode-select'
import { act, cleanup, render, waitFor } from '@testing-library/react'
import { StrictMode, useState } from 'react'
import { afterEach, expect, it, vi } from 'vitest'
import { WorktreeModeSelect } from './mode-select'

vi.mock('dsh-tauri-ui/client', () => ({
  Chip: ({ children }: { children: ReactNode }) => <button type="button">{children}</button>,
  Menu: ({ anchor }: { anchor: ReactNode }) => anchor,
  Icon: () => null,
  ChevronDown: () => null,
  CircleTree: () => null,
}))
vi.mock('dsh-tauri/client', async () => {
  const { forEach, get } = await import('lodash-es')
  return { forEach, get }
})
vi.mock('../hooks/use-worktree-session', () => ({ useWorktreeSession: () => ({ isGit: true, mode: 'local' }) }))
vi.mock('../hooks/use-waiter', () => ({ useWaiter: () => ({ mountedRef: { current: true }, wait: vi.fn() }) }))
vi.mock('../locales', () => ({ locale: { text: (key: string) => key, useLocale: () => 'en' } }))
vi.mock('../store', () => ({ store: { worktree: { patch: vi.fn() } } }))
vi.mock('../service/worktree', () => ({ attach: vi.fn(), create: vi.fn() }))

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

it('moves one portal with the composer target, removes it when the target disappears, and cleans up on unmount', async () => {
  const props: ModeSelectProps = {
    sessionId: 'one',
    useInput(selector) {
      const [input] = useState({ draft: '' })
      return selector(input)
    },
    inputActions: { setDraft: vi.fn(), submit: vi.fn() },
    sessionsRuntime: {} as ModeSelectProps['sessionsRuntime'],
    workspacesRuntime: {} as ModeSelectProps['workspacesRuntime'],
    resolveAttachments: () => undefined,
  }
  function surface(sessionId: string, target: boolean) {
    return (
      <StrictMode>
        <section data-composer-seat>
          {target && <div data-slot="conversation.hero.agentPreset">preset</div>}
          <WorktreeModeSelect {...props} sessionId={sessionId} />
        </section>
      </StrictMode>
    )
  }
  const view = render(surface('one', true))
  const selector = '[data-dsh-tauri-worktree-mode]'
  const target = view.container.querySelector('[data-slot]')!
  const host = view.container.querySelector(selector)
  expect(host).not.toBeNull()
  expect(target.nextElementSibling).toBe(host)
  expect(host?.textContent).toBe('modeLocal')
  const sibling = document.createElement('span')
  await act(async () => {
    target.after(sibling)
  })
  await waitFor(() => expect(target.nextElementSibling).toBe(host))
  expect(view.container.querySelectorAll(selector)).toHaveLength(1)
  view.rerender(surface('two', true))
  expect(host?.isConnected).toBe(false)
  expect(view.container.querySelector(selector)?.getAttribute('data-dsh-tauri-worktree-mode')).toBe('two')
  view.rerender(surface('two', false))
  await waitFor(() => expect(view.container.querySelector(selector)).toBeNull())
  view.rerender(surface('two', true))
  await waitFor(() => expect(view.container.querySelector(selector)?.textContent).toBe('modeLocal'))
  const finalHost = view.container.querySelector(selector)!
  view.unmount()
  expect(finalHost.isConnected).toBe(false)
})
