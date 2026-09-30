import { describe, expect, it } from 'vitest'
import { eventEntriesOf, machineRowsOf } from './machines'

describe('machineRowsOf', () => {
  it('合并 items 与 discovered、按名排序并丢掉坏行', () => {
    const { enabled, machines } = machineRowsOf({
      enabled: true,
      items: [
        { id: 'b', name: 'beta', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4002', color: '#123456', tintBorder: true },
        { id: 'a', name: 'alpha', state: 'disconnected', lastError: 'boom', authMethod: 'key', nextRetryAt: 1725000000000 },
        { id: '', name: 'bad', state: 'connected' },
        { id: 'x' },
        { id: 'c', name: 'gamma', state: 'nonsense' },
        null,
      ],
      discovered: [{ id: 'alias', name: 'alias-host', state: 'disconnected' }],
    })

    expect(enabled).toBe(true)
    expect(machines.map(row => row.id)).toEqual(['alias', 'a', 'b'])
    expect(machines[2]).toMatchObject({ color: '#123456', tintBorder: true, tunnelBaseUrl: 'http://127.0.0.1:4002' })
    expect(machines[1]).toMatchObject({ authMethod: 'key', nextRetryAt: 1725000000000, lastError: 'boom' })
  })

  it('未启用或载荷缺失都退化为空列表', () => {
    expect(machineRowsOf({ enabled: false, items: [], discovered: [] })).toEqual({ enabled: false, machines: [] })
    expect(machineRowsOf(undefined)).toEqual({ enabled: false, machines: [] })
    expect(machineRowsOf({ enabled: true })).toEqual({ enabled: true, machines: [] })
  })

  it('progress 白名单外的阶段整块丢弃', () => {
    const { machines } = machineRowsOf({
      enabled: true,
      items: [
        { id: 'm1', name: 'm1', state: 'connecting', progress: { phase: 'bogus' } },
        { id: 'm2', name: 'm2', state: 'connecting', progress: { phase: 'installing', attempt: 2 } },
      ],
    })

    expect(machines[0].progress).toBeUndefined()
    expect(machines[1].progress).toEqual({ phase: 'installing', attempt: 2 })
  })
})

describe('eventEntriesOf', () => {
  it('seq/line 齐备才收', () => {
    expect(eventEntriesOf({
      items: [{ seq: 1, line: 'a' }, { seq: 'x', line: 'b' }, { line: 'c' }, null, { seq: 2, line: 'd' }],
    })).toEqual([{ seq: 1, line: 'a' }, { seq: 2, line: 'd' }])
    expect(eventEntriesOf(undefined)).toEqual([])
  })
})
