import type { DshWebviewElement, WebviewIpcMessageEvent } from '@/types/webview'
import { describe, expect, it } from 'vitest'
import { postToGuest, readGuestMessage } from './webview-bridge'

function ipcEvent(payload: unknown, channel = 'dsh-desktop'): WebviewIpcMessageEvent {
  return { channel, args: [payload] } as unknown as WebviewIpcMessageEvent
}

function fakeGuest(code: string[]): DshWebviewElement {
  return {
    executeJavaScript: async (script: string) => {
      code.push(script)
      return null
    },
  } as unknown as DshWebviewElement
}

describe('readGuestMessage', () => {
  it('把 guest 发来的 JSON 字符串还原为协议消息', () => {
    expect(readGuestMessage(ipcEvent(JSON.stringify({ type: 'dsh://sidebar:collapsed', collapsed: true })))).toEqual({
      type: 'dsh://sidebar:collapsed',
      collapsed: true,
    })
  })

  it('直接收到对象载荷时原样返回', () => {
    expect(readGuestMessage(ipcEvent({ type: 'dsh://plugin-boot:ready' }))).toEqual({ type: 'dsh://plugin-boot:ready' })
  })

  it('丢弃没有 type 的载荷', () => {
    expect(readGuestMessage(ipcEvent(JSON.stringify({ collapsed: true })))).toBeNull()
  })

  it('丢弃非法 JSON 与标量载荷', () => {
    expect(readGuestMessage(ipcEvent('{not json'))).toBeNull()
    expect(readGuestMessage(ipcEvent(42))).toBeNull()
  })
})

describe('postToGuest', () => {
  it('宿主 → guest 走 executeJavaScript，并补上 dsh-desktop 来源标识', () => {
    const code: string[] = []
    postToGuest(fakeGuest(code), { type: 'dsh://sidebar:toggle' })
    expect(code).toHaveLength(1)
    expect(code[0]).toContain('window.postMessage(')
    expect(code[0]).toContain('"source":"dsh-desktop"')
    expect(code[0]).toContain('"type":"dsh://sidebar:toggle"')
  })

  it('guest 未挂载时静默丢弃', () => {
    expect(() => postToGuest(null, { type: 'dsh://sidebar:toggle' })).not.toThrow()
  })
})
