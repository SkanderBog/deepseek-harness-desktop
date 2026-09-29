import { beforeEach, describe, expect, it } from 'vitest'
import { clearTurnEndFacts, turnEndFact } from '../service/turn-end'
import { handleSessionEvent } from './session-event'

describe('handleSessionEvent', () => {
  beforeEach(() => {
    clearTurnEndFacts()
  })

  it('记下 turn/end 的结束原因与回合号', () => {
    handleSessionEvent({ id: 's1' }, { type: 'turn/end', data: { turn: 3, reason: { kind: 'aborted' } } })
    expect(turnEndFact('s1')).toEqual({ reason: 'aborted', turn: 3 })
  })

  it('忽略其它事件类型', () => {
    handleSessionEvent({ id: 's1' }, { type: 'turn/start', data: { turn: 4, reason: { kind: 'aborted' } } })
    expect(turnEndFact('s1')).toBeUndefined()
  })

  it('缺少会话 id 或结束原因时不记录', () => {
    handleSessionEvent({}, { type: 'turn/end', data: { turn: 1, reason: { kind: 'completed' } } })
    handleSessionEvent({ id: 's1' }, { type: 'turn/end', data: { turn: 1 } })
    handleSessionEvent({ id: 's1' }, { type: 'turn/end' })
    expect(turnEndFact('s1')).toBeUndefined()
  })

  it('宿主没给回合号时用 -1 占位', () => {
    handleSessionEvent({ id: 's2' }, { type: 'turn/end', data: { reason: { kind: 'completed' } } })
    expect(turnEndFact('s2')).toEqual({ reason: 'completed', turn: -1 })
  })

  it('同一会话只保留最近一次结束事实', () => {
    handleSessionEvent({ id: 's1' }, { type: 'turn/end', data: { turn: 1, reason: { kind: 'completed' } } })
    handleSessionEvent({ id: 's1' }, { type: 'turn/end', data: { turn: 2, reason: { kind: 'aborted' } } })
    expect(turnEndFact('s1')).toEqual({ reason: 'aborted', turn: 2 })
  })
})
