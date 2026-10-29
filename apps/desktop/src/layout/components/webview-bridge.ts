import type { RefObject } from 'react'
import type { DshWebviewElement, WebviewIpcMessageEvent } from '@/types/webview'
import { useEffect, useRef } from 'react'

export const WEBVIEW_HOST_SOURCE = 'dsh-desktop'

export interface WebviewOutboundMessage {
  type?: string
  [key: string]: unknown
}

export interface GuestBridgeMessage {
  type?: string
  id?: string
  detail?: string
  collapsed?: boolean
  rows?: unknown
  [key: string]: unknown
}

type GuestListener = (message: GuestBridgeMessage) => void

const guestListeners = new Set<GuestListener>()

export function emitGuestMessage(message: GuestBridgeMessage): void {
  for (const listener of guestListeners)
    listener(message)
}

function subscribeGuestMessage(listener: GuestListener): () => void {
  guestListeners.add(listener)
  return () => {
    guestListeners.delete(listener)
  }
}

export function useGuestMessage(handler: GuestListener): void {
  const handlerRef = useRef(handler)
  handlerRef.current = handler

  // keep:effect 订阅 guest 协议总线，卸载时必须注销
  useEffect(() => subscribeGuestMessage(message => handlerRef.current(message)), [])
}

function guestTargetOrigin(guest: DshWebviewElement): string {
  try {
    const url = new URL(guest.getURL())
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.origin : '*'
  }
  catch {
    return '*'
  }
}

export function postToGuest(guest: DshWebviewElement | null, message: WebviewOutboundMessage): void {
  if (!guest)
    return
  const payload = JSON.stringify({ source: WEBVIEW_HOST_SOURCE, ...message })
  const target = JSON.stringify(guestTargetOrigin(guest))
  void guest.executeJavaScript(`window.postMessage(${payload}, ${target})`)
    .catch((error: unknown) => console.warn('[webview] post to guest failed:', error))
}

// ponytail: 频道名不设防，只认载荷里的 dsh:// 协议；guest 侧发送方在插件侧，频道名以它为准
export function readGuestMessage(event: WebviewIpcMessageEvent): GuestBridgeMessage | null {
  const payload = event.args?.[0]
  if (typeof payload === 'string') {
    try {
      const parsed: unknown = JSON.parse(payload)
      return isGuestMessage(parsed) ? parsed : null
    }
    catch {
      return null
    }
  }
  return isGuestMessage(payload) ? payload : null
}

function isGuestMessage(value: unknown): value is GuestBridgeMessage {
  return value !== null && typeof value === 'object' && typeof (value as { type?: unknown }).type === 'string'
}

export interface GuestEventHandlers {
  onMessage: (message: GuestBridgeMessage) => void
  onDomReady: () => void
  onFinishLoad: () => void
  onFailLoad: () => void
}

export function useGuestElementEvents(
  guestRef: RefObject<DshWebviewElement | null>,
  renderKey: string,
  attached: boolean,
  handlers: GuestEventHandlers,
): void {
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers

  // keep:effect 订阅 <webview> 元素事件（Electron 只在元素实例上派发，逐次挂载取新节点）
  useEffect(() => {
    const guest = guestRef.current
    if (!guest)
      return
    const onIpc = (event: Event) => {
      const message = readGuestMessage(event as WebviewIpcMessageEvent)
      if (message)
        handlersRef.current.onMessage(message)
    }
    const onDomReady = () => handlersRef.current.onDomReady()
    const onFinishLoad = () => handlersRef.current.onFinishLoad()
    const onFailLoad = () => handlersRef.current.onFailLoad()

    guest.addEventListener('ipc-message', onIpc)
    guest.addEventListener('dom-ready', onDomReady)
    guest.addEventListener('did-finish-load', onFinishLoad)
    guest.addEventListener('did-fail-load', onFailLoad)
    return () => {
      guest.removeEventListener('ipc-message', onIpc)
      guest.removeEventListener('dom-ready', onDomReady)
      guest.removeEventListener('did-finish-load', onFinishLoad)
      guest.removeEventListener('did-fail-load', onFailLoad)
    }
  }, [guestRef, renderKey, attached])
}
