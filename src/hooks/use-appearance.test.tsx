// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react'
import { defineStore } from 'valtio-define'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useAppearance } from './use-appearance'

const mocks = vi.hoisted(() => ({ setting: {} as any }))
vi.mock('@/store', () => ({ store: { get setting() {
  return mocks.setting
} } }))
vi.mock('./use-dsh-style', () => ({ useDshStyle: () => [{ colorScheme: 'dark' }] }))

afterEach(() => {
  cleanup()
  document.body.replaceChildren()
  document.head.replaceChildren()
  delete (window as any).__DSH_TRANSPARENT__
  vi.restoreAllMocks()
})

function setup(transparent: boolean) {
  ;(window as any).__DSH_TRANSPARENT__ = transparent
  mocks.setting = defineStore({ state: () => ({ appearance: { palette: 'nord', terminal: false, opacity: 70 } }) })
  const iframe = document.createElement('iframe')
  iframe.src = 'http://localhost:3080'
  document.body.append(iframe)
  const post = vi.spyOn(iframe.contentWindow!, 'postMessage').mockImplementation(() => {})
  const hook = renderHook(() => useAppearance({ current: iframe }))
  function ready(origin = 'http://localhost:3080') {
    act(() => window.dispatchEvent(new MessageEvent('message', {
      source: iframe.contentWindow!,
      origin,
      data: { type: 'dsh://appearance:ready' },
    })))
  }
  return { post, hook, ready }
}

describe('desktop appearance projection', () => {
  it('keeps the canvas opaque until the native window has restarted with transparency enabled', () => {
    const { post, ready, hook } = setup(false)
    ready()
    expect(post.mock.calls.at(-1)?.[0]).toMatchObject({ type: 'dsh://appearance', appearance: { opacity: 100 } })
    expect(hook.result.current).toContain('#343c4a 100%')
  })

  it('sends the saved opacity to a transparent window and removes the bridge listener on unmount', () => {
    const { post, ready, hook } = setup(true)
    ready()
    expect(post.mock.calls.at(-1)?.[0]).toMatchObject({ appearance: { palette: 'nord', opacity: 70 } })
    expect(hook.result.current).toContain('#343c4a 70%')
    hook.unmount()
    post.mockClear()
    ready()
    expect(post).not.toHaveBeenCalled()
  })

  it('rejects an appearance handshake from the wrong origin', () => {
    const { post, ready } = setup(true)
    ready('https://example.invalid')
    expect(post).not.toHaveBeenCalled()
  })

  it('does no message work on unrelated renders and clears styling on reset', async () => {
    const { post, ready, hook } = setup(true)
    ready()
    post.mockClear()
    hook.rerender()
    expect(post).not.toHaveBeenCalled()
    await act(async () => {
      mocks.setting.appearance = { palette: 'default', terminal: false, opacity: 100 }
    })
    expect(post.mock.calls.at(-1)?.[0]).toMatchObject({ appearance: { palette: 'default', opacity: 100 } })
    expect(hook.result.current).toBe('')
  })
})
