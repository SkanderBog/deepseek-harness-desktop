import type { NotificationSettings } from '../types'
import { describe, expect, it } from 'vitest'
import { allowPendingNotification, allowTurnNotification, boundText, notificationTag } from './decision'

const settings: NotificationSettings = {
  turnComplete: 'background',
  approval: true,
  question: false,
  sound: 'default',
  customSound: null,
}

/** 宿主 `NOTIFICATION_SHIM_JS` 里 `sessionIdFromTag` 使用的同一条正则。 */
const SHIM_TAG_RE = /^dsh-notification-(?:pending-)?(.+)-\d+$/

describe('allowTurnNotification', () => {
  const base = { mode: 'background' as const, hostHidden: false, sessionId: 's1', currentSessionId: 's1' }

  it('never 模式一律不提醒', () => {
    expect(allowTurnNotification({ ...base, mode: 'never', hostHidden: true })).toBe(false)
    expect(allowTurnNotification({ ...base, mode: 'never', currentSessionId: 's2' })).toBe(false)
  })

  it('always 模式一律提醒', () => {
    expect(allowTurnNotification({ ...base, mode: 'always' })).toBe(true)
    expect(allowTurnNotification({ ...base, mode: 'always', currentSessionId: undefined })).toBe(true)
  })

  it('background 在窗口隐藏时提醒', () => {
    expect(allowTurnNotification({ ...base, hostHidden: true })).toBe(true)
  })

  it('background 在用户切到别的会话时提醒', () => {
    expect(allowTurnNotification({ ...base, currentSessionId: 's2' })).toBe(true)
  })

  it('background 在窗口可见且仍盯着同一会话时不提醒', () => {
    expect(allowTurnNotification(base)).toBe(false)
  })

  it('background 在拿不到当前会话时仍提醒：只有确知「就停在这个会话上」才抑制', () => {
    // 与参考实现 `source/dsh-notification` 的 shouldShow 一致：读不到当前会话（会话投影
    // 缺席、或界面还没打开任何会话）时无法证明用户正盯着它，宁可多提醒一次。
    expect(allowTurnNotification({ ...base, currentSessionId: undefined })).toBe(true)
  })
})

describe('allowPendingNotification', () => {
  it('按类别读取各自的开关', () => {
    expect(allowPendingNotification(settings, 'approval')).toBe(true)
    expect(allowPendingNotification(settings, 'question')).toBe(false)
    expect(allowPendingNotification({ ...settings, approval: false, question: true }, 'approval')).toBe(false)
    expect(allowPendingNotification({ ...settings, approval: false, question: true }, 'question')).toBe(true)
  })
})

describe('notificationTag', () => {
  it('轮次完成 tag 可被宿主演解回 sessionId', () => {
    expect(notificationTag('session-abc', 'turn', 3)).toBe('dsh-notification-session-abc-3')
    expect('dsh-notification-session-abc-3'.match(SHIM_TAG_RE)?.[1]).toBe('session-abc')
  })

  it('待处理交互 tag 同样可被宿主演解回 sessionId', () => {
    expect(notificationTag('session-abc', 'approval', 7)).toBe('dsh-notification-pending-session-abc-7')
    expect('dsh-notification-pending-session-abc-7'.match(SHIM_TAG_RE)?.[1]).toBe('session-abc')
  })

  it('sessionId 含连字符或数字时仍能反解（贪婪匹配回溯到最后一节序号）', () => {
    const tag = notificationTag('a-1-b', 'turn', 12)
    expect(tag.match(SHIM_TAG_RE)?.[1]).toBe('a-1-b')
  })
})

describe('boundText', () => {
  it('去除首尾空白', () => {
    expect(boundText('  已完成  ', 400)).toBe('已完成')
  })

  it('未超长时原样返回', () => {
    expect(boundText('abc', 3)).toBe('abc')
  })

  it('超长时截断并补省略号，总长等于上限', () => {
    const bounded = boundText('x'.repeat(50), 10)
    expect(bounded).toBe(`${'x'.repeat(9)}…`)
    expect(bounded).toHaveLength(10)
  })
})
