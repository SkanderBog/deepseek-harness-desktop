// @vitest-environment jsdom
import type { PetBubbleHandle, PetBubbleOptions, PetRef } from 'dsh-pet-component'
import { act, cleanup, render, renderHook, screen, waitFor } from '@testing-library/react'
import { createElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Hint } from '@/ui/pet/hint'
import { petI18n, setPetLanguage } from '../i18n'
import { createBubbleTracker, useBubbleTracker } from './use-bubble-tracker'
import { bubbleContent, getLiveActivity, statusCopy } from './use-bubble-tracker.helpers'

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    native.listeners.set(event, handler)
    return Promise.resolve(() => {
      native.listeners.delete(event)
    })
  }),
}))

function makeBubble() {
  const bubble = Object.assign(vi.fn((options: PetBubbleOptions) => options.id ?? 'bubble'), { close: vi.fn(), clear: vi.fn() })
  return bubble as typeof bubble & PetBubbleHandle
}

beforeEach(() => {
  document.documentElement.lang = 'zh-CN'
  native.invoke.mockReset()
  native.listeners.clear()
  setPetLanguage('en')
})
afterEach(() => {
  cleanup()
  setPetLanguage('en')
  vi.restoreAllMocks()
})

describe('pet language regression', () => {
  it('updates the unavailable-pet hint with the shared pet locale', () => {
    render(createElement(Hint, { petId: 'cat' }))
    expect(screen.getByText('Pet “cat” is unavailable — pick another one in Settings → Pets').textContent)
      .toBe('Pet “cat” is unavailable — pick another one in Settings → Pets')
    act(() => setPetLanguage('ko'))
    expect(screen.getByText('펫 「cat」의 리소스를 사용할 수 없습니다. 설정 → 펫에서 다시 선택해 주세요.').textContent)
      .toBe('펫 「cat」의 리소스를 사용할 수 없습니다. 설정 → 펫에서 다시 선택해 주세요.')
  })

  it('uses English approval and thinking copy despite the HTML language', () => {
    document.documentElement.lang = 'zh-CN'
    expect(bubbleContent({ id: '0', phase: 'approval' }, 'waiting'))
      .toEqual({ title: 'Needs approval · New session', description: 'Awaiting your approval', loading: false })
    expect(getLiveActivity({ id: '0', liveActivity: { kind: 'reasoning', text: 'Checking code' } }, 'thinking'))
      .toBe('Thought · Checking code')
    expect(bubbleContent({ id: '0', task: 'Fix login' }, 'working').description).toBe('Working on “Fix login”')
    expect(getLiveActivity({ id: '0', liveActivity: { kind: 'tool', name: 'read', args: '{"path":"a.ts"}' } }, 'working'))
      .toBe('Read · a.ts')
  })

  it.each([
    ['en-US', 'New session', 'Analyzing information', 'Awaiting your approval'],
    ['zh-CN', '新会话', '正在分析', '等待审批'],
    ['ko-KR', '새 세션', '정보 분석 중', '승인 대기 중'],
  ])('resolves all work status groups at display time in %s', (locale, title, thinking, approval) => {
    setPetLanguage(locale)
    expect(bubbleContent({ id: '0' }, 'thinking')).toEqual({ title, description: thinking, loading: true })
    expect(statusCopy('approval', 0)).toBe(approval)
    for (const group of ['thinking', 'working', 'waiting', 'result', 'success', 'error', 'review', 'failed', 'running', 'stopped']) {
      for (const seed of [0, 1, 2]) {
        const copy = statusCopy(group, seed)
        expect(copy).not.toContain('pet.bubble.')
        expect(copy.length).toBeGreaterThan(0)
      }
    }
  })

  it('preserves error and task precedence over default status copy', () => {
    expect(bubbleContent({ id: '0', task: 'custom task', lastAgentError: 'custom error' }, 'error').description)
      .toBe('Failed: custom error')
    expect(bubbleContent({ id: '0', task: 'custom task', liveActivity: { kind: 'reasoning', text: 'custom thought' } }, 'thinking').description)
      .toBe('Working on “custom task”')
  })

  it('refreshes an existing approval bubble when native settings change', async () => {
    native.invoke.mockResolvedValue({ language: 'en' })
    const bubble = makeBubble()
    renderHook(() => useBubbleTracker({ bubble } as unknown as PetRef))
    await waitFor(() => expect(native.invoke).toHaveBeenCalledWith('get_app_config'))
    act(() => native.listeners.get('session:create')!({ payload: { id: '0', phase: 'approval', workStatus: 'waiting' } }))
    expect(bubble.mock.calls.at(-1)![0].title).toBe('Needs approval · New session')
    act(() => native.listeners.get('setting_updated')!({ payload: { language: 'ko' } }))
    expect(bubble.mock.calls.at(-1)![0]).toMatchObject({ id: '0', title: '승인 필요 · 새 세션', description: '승인 대기 중' })
    expect(document.documentElement.lang).toBe('ko-KR')
    expect(native.invoke.mock.calls).toEqual([['get_app_config']])
  })

  it.each([
    ['zh-CN', '需授权 · 新会话', '等待审批'],
    ['ko-KR', '승인 필요 · 새 세션', '승인 대기 중'],
  ])('falls back to %s when the native preference cannot be loaded', async (locale, title, description) => {
    vi.spyOn(navigator, 'language', 'get').mockReturnValue(locale)
    const error = new Error('settings unavailable')
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    native.invoke.mockRejectedValue(error)
    const bubble = makeBubble()
    renderHook(() => useBubbleTracker({ bubble } as unknown as PetRef))
    act(() => native.listeners.get('session:create')!({ payload: { id: '0', phase: 'approval', workStatus: 'waiting' } }))
    await waitFor(() => expect(warn).toHaveBeenCalledWith('[pet] failed to load language:', error))
    expect(petI18n.language).toBe(locale)
    expect(bubble.mock.calls.at(-1)![0]).toMatchObject({ id: '0', title, description })
  })

  it('ignores a failed initial preference after a newer language event', async () => {
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('zh-CN')
    let reject!: (reason: Error) => void
    native.invoke.mockReturnValue(new Promise((_resolve, fail) => {
      reject = fail
    }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = new Error('settings unavailable')
    const bubble = makeBubble()
    renderHook(() => useBubbleTracker({ bubble } as unknown as PetRef))
    act(() => native.listeners.get('session:create')!({ payload: { id: '0', phase: 'approval', workStatus: 'waiting' } }))
    act(() => native.listeners.get('setting_updated')!({ payload: { language: 'ko' } }))
    const count = bubble.mock.calls.length
    await act(async () => reject(error))
    expect(warn).toHaveBeenCalledWith('[pet] failed to load language:', error)
    expect(petI18n.language).toBe('ko-KR')
    expect(bubble).toHaveBeenCalledTimes(count)
  })

  it('ignores a failed initial preference after unmount', async () => {
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('zh-CN')
    let reject!: (reason: Error) => void
    native.invoke.mockReturnValue(new Promise((_resolve, fail) => {
      reject = fail
    }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = new Error('settings unavailable')
    const bubble = makeBubble()
    const { unmount } = renderHook(() => useBubbleTracker({ bubble } as unknown as PetRef))
    unmount()
    await act(async () => reject(error))
    expect(warn).toHaveBeenCalledWith('[pet] failed to load language:', error)
    expect(petI18n.language).toBe('en-US')
    expect(bubble).not.toHaveBeenCalled()
    expect(native.listeners.size).toBe(0)
  })

  it('ignores an initial preference that arrives after a newer language event', async () => {
    let resolve!: (value: { language: string }) => void
    native.invoke.mockReturnValue(new Promise((done) => {
      resolve = done
    }))
    const bubble = makeBubble()
    const { unmount } = renderHook(() => useBubbleTracker({ bubble } as unknown as PetRef))
    act(() => native.listeners.get('setting_updated')!({ payload: { language: 'ko' } }))
    await act(async () => resolve({ language: 'zh' }))
    expect(petI18n.language).toBe('ko-KR')
    unmount()
    await waitFor(() => expect(native.listeners.size).toBe(0))
  })

  it('translates subsequent updates and one-shot completion without replaying terminal bubbles', () => {
    const bubble = makeBubble()
    const tracker = createBubbleTracker(bubble)
    tracker.apply({ id: '0', status: 'running' }, 'create')
    setPetLanguage('ko')
    tracker.flush()
    expect(bubble.mock.calls.at(-1)![0].description).toBe('작업 실행 중')
    tracker.apply({ id: '0' }, 'update')
    expect(bubble.mock.calls.at(-1)![0]).toMatchObject({ id: '0:done', description: '완료', timeout: 3000 })
    tracker.apply({ id: '1', workStatus: 'success' }, 'create')
    const count = bubble.mock.calls.length
    setPetLanguage('en')
    tracker.flush()
    expect(bubble).toHaveBeenCalledTimes(count)
  })
})
