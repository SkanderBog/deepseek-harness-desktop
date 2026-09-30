import type { SshApiClient } from '../src/store/modules/remote/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// 本地实例不可达时的降级轮询：`refresh` 每 2 秒跑一轮，实例停着（插件操作会停服）时会
// 连续失败几百轮。这里锁定「一轮降级只留一条日志」——每轮都带堆栈打一遍会把日志淹掉，
// 而这行信息的增量是零。恢复之后再次不可达要能重新打一次，否则第二次故障看不见。

const { bindSshApiForTests, remote } = await import('../src/store/modules/remote/store')

const unreachable = (): Promise<never> => Promise.reject(new TypeError('Failed to fetch'))

/** 只替换数据面：`bindSshApiForTests` 还要求 connect/disconnect（这些用例走不到）。 */
function onlyListMachines(listMachines: SshApiClient['listMachines']) {
  return {
    listMachines,
    connect: async () => ({ tunnelBaseUrl: '' }),
    disconnect: async () => {},
  }
}

let warn: ReturnType<typeof vi.spyOn>

beforeEach(() => {
  warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  remote.available = true
  remote.refreshing = false
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('remote poll logging', () => {
  it('logs an unreachable instance once per outage, not once per poll', async () => {
    bindSshApiForTests(onlyListMachines(unreachable))

    await remote.refresh()
    await remote.refresh()
    await remote.refresh()

    expect(warn).toHaveBeenCalledTimes(1)
    expect(remote.available).toBe(false)
  })

  it('logs again when a fresh outage follows a recovery', async () => {
    bindSshApiForTests(onlyListMachines(unreachable))
    await remote.refresh()
    await remote.refresh()

    bindSshApiForTests(onlyListMachines(async () => ({ enabled: false, machines: [] })))
    await remote.refresh()
    expect(remote.available).toBe(true)
    expect(warn).toHaveBeenCalledTimes(1)

    bindSshApiForTests(onlyListMachines(unreachable))
    await remote.refresh()

    expect(warn).toHaveBeenCalledTimes(2)
  })

  it('does not log when the instance is reachable but has no SSH API', async () => {
    const { SshApiHttpError } = await import('../src/apis/http')
    bindSshApiForTests(onlyListMachines(async () => {
      throw new SshApiHttpError(404)
    }))

    await remote.refresh()

    // 可达但没有 API 是「SSH 未启用」，不是不可达：不该产生降级日志
    expect(warn).not.toHaveBeenCalled()
    expect(remote.available).toBe(true)
    expect(remote.enabled).toBe(false)
  })
})
