import { afterEach, describe, expect, it, vi } from 'vitest'
import { activeQueues as queues, toast } from '../../src/utils/toast'

/**
 * HeroUI 3.2 的 toast 队列把关闭拆成两步：`close()` 先把条目标记为退出并同步
 * 排队 `onClose`（业务层用 microtask 延后），条目仍留在 `visibleToasts` 里播放
 * 退出动画，动画计时结束后才真正移除。断言因此要分三段：同步只改状态，
 * microtask 后才回调，计时走完才消失。
 */
async function closeAndSettle(key: string, queue: typeof queues['top']): Promise<void> {
  queue.close(key)
  await vi.advanceTimersByTimeAsync(1000)
}

afterEach(async () => {
  // clear() 同样走退出动画：先推进计时把条目真正移除，再切回真实计时器，
  // 否则残留条目会串到下一个用例。
  toast.clear()
  await vi.advanceTimersByTimeAsync(1000)
  vi.useRealTimers()
})

describe('toast lifecycle', () => {
  it('keeps persistent notifications open until explicitly closed and runs their callback', async () => {
    vi.useFakeTimers()
    const onClose = vi.fn()
    const key = toast('Update available', { timeout: 0, onClose })
    const queue = queues['bottom end']
    const notification = queue.visibleToasts.find(item => item.key === key)!

    expect(notification.timer).toBeUndefined()
    expect(notification.content).not.toHaveProperty('onClose')
    expect(notification.content).not.toHaveProperty('timeout')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(queue.visibleToasts.some(item => item.key === key)).toBe(true)
    expect(onClose).not.toHaveBeenCalled()

    await closeAndSettle(key, queue)
    expect(onClose).toHaveBeenCalledOnce()
    expect(queue.visibleToasts.some(item => item.key === key)).toBe(false)
  })

  it('expires a notification using its requested timeout', async () => {
    vi.useFakeTimers()
    const onClose = vi.fn()
    const key = toast('Update available', { timeout: 8000, onClose })
    const queue = queues['bottom end']
    const notification = queue.visibleToasts.find(item => item.key === key)!

    notification.timer!.resume()
    await vi.advanceTimersByTimeAsync(7999)
    expect(queue.visibleToasts.some(item => item.key === key)).toBe(true)
    expect(onClose).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1 + 1000)
    expect(queue.visibleToasts.some(item => item.key === key)).toBe(false)
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('closes only the selected notification in its own placement', async () => {
    vi.useFakeTimers()
    const onClose = vi.fn()
    const key = toast('Download complete', { placement: 'top', onClose })
    const otherKey = toast('Update available', { timeout: 0 })

    await closeAndSettle(key, queues.top)

    expect(queues.top.visibleToasts.some(item => item.key === key)).toBe(false)
    expect(queues['bottom end'].visibleToasts.some(item => item.key === otherKey)).toBe(true)
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('never discards an actionable notification when the queue overflows', async () => {
    vi.useFakeTimers()
    const keys = [1, 2, 3, 4].map(index => toast(`Authorise ${index}`, { timeout: 0, sticky: true }))
    const queue = queues['bottom end']

    await Promise.resolve()
    expect(toast.isActive(keys[0])).toBe(true)

    await closeAndSettle(keys[3], queue)
    expect(queue.visibleToasts.map(item => item.key)).toEqual([keys[2], keys[1], keys[0]])
  })

  it('still evicts the oldest ordinary notification when the queue overflows', async () => {
    vi.useFakeTimers()
    const onClose = vi.fn()
    const first = toast('Result 1', { timeout: 0, onClose })
    toast('Result 2', { timeout: 0 })
    toast('Result 3', { timeout: 0 })
    toast('Result 4', { timeout: 0 })

    await Promise.resolve()
    await Promise.resolve()
    expect(onClose).toHaveBeenCalledWith('evicted')
    expect(toast.isActive(first)).toBe(false)
  })
})
