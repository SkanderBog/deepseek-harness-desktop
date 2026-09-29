import { afterEach, describe, expect, it } from 'vitest'
import { activeQueues, toast } from '../src/utils/toast'

interface StatelyQueueProbe {
  queue: Array<{ key: string, content?: Record<string, unknown> }>
  visibleToasts: unknown[]
}

function probe(): StatelyQueueProbe {
  return activeQueues['bottom end'].getQueue() as unknown as StatelyQueueProbe
}

function entryOf(key: string) {
  return probe().queue.find(item => item.key === key)
}

afterEach(() => {
  toast.clear()
})

describe('toast.update', () => {
  it('writes the update through to the queue entry so the default bubble re-renders', () => {
    const key = toast('正在升级 aaa...', { timeout: 0 })
    const visibleBefore = probe().visibleToasts

    expect(entryOf(key)?.content).toMatchObject({ title: '正在升级 aaa...' })

    toast.update(key, { title: '正在升级 2 个插件...', isLoading: true, description: 'added 1 package' })

    expect(entryOf(key)?.content).toMatchObject({
      title: '正在升级 2 个插件...',
      isLoading: true,
      description: 'added 1 package',
    })
    expect(probe().visibleToasts).not.toBe(visibleBefore)
  })

  it('ignores keys that were already closed', () => {
    const key = toast('x', { timeout: 0 })
    toast.close(key)

    expect(() => toast.update(key, { description: 'late line' })).not.toThrow()
    expect(entryOf(key)).toBeUndefined()
  })
})
