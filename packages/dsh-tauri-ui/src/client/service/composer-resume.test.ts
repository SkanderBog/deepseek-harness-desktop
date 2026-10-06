import { beforeEach, describe, expect, it, vi } from 'vitest'
import { resumeComposer } from './composer-resume'

const mocks = vi.hoisted(() => ({
  postSessionResume: vi.fn(async () => ({})),
}))

vi.mock('../apis', () => ({ postSessionResume: mocks.postSessionResume }))

beforeEach(() => {
  mocks.postSessionResume.mockClear()
})

describe('resumeComposer', () => {
  it('keeps the ordinary resume request on the original session', async () => {
    await expect(resumeComposer({ sessionId: 'source' })).resolves.toEqual({ ok: true })
    expect(mocks.postSessionResume).toHaveBeenCalledWith({ sessionId: 'source' })
  })

  it('forks at the safe boundary, opens the child and resumes only that child', async () => {
    const fork = vi.fn(async () => 'child')
    const opened = Promise.resolve()
    const open = vi.fn(() => ({ status: 'opened' as const, value: opened }))

    await expect(resumeComposer({
      sessionId: 'source',
      recovery: { atSeq: 7, sessions: { fork }, navigation: { open } },
    })).resolves.toEqual({ ok: true })

    expect(fork).toHaveBeenCalledWith({ sessionId: 'source', atSeq: 7, increaseTitle: true })
    expect(open).toHaveBeenCalledWith('child')
    expect(mocks.postSessionResume).toHaveBeenCalledWith({ sessionId: 'child', recoverFromSessionId: 'source' })
  })

  it('does not resume either session when the recovery child cannot be opened', async () => {
    const fork = vi.fn(async () => 'child')
    const open = vi.fn(() => ({ status: 'unavailable' as const, reason: 'navigation unavailable' }))

    await expect(resumeComposer({
      sessionId: 'source',
      recovery: { atSeq: 7, sessions: { fork }, navigation: { open } },
    })).resolves.toEqual({ ok: false, error: 'navigation unavailable' })

    expect(mocks.postSessionResume).not.toHaveBeenCalled()
  })
})
