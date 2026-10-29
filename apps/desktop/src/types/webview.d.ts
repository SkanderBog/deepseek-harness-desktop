import type { DetailedHTMLProps, HTMLAttributes } from 'react'

export interface WebviewIpcMessageEvent extends Event {
  channel?: string
  args?: unknown[]
}

export interface WebviewFailLoadEvent extends Event {
  errorCode?: number
  errorDescription?: string
  validatedURL?: string
  isMainFrame?: boolean
}

export interface DshWebviewElement extends HTMLElement {
  src: string
  executeJavaScript: (code: string, userGesture?: boolean) => Promise<unknown>
  send: (channel: string, ...args: unknown[]) => void
  loadURL: (url: string) => Promise<void>
  reload: () => void
  stop: () => void
  getURL: () => string
}

declare module 'react' {
  namespace JSX {
    interface IntrinsicElements {
      webview: DetailedHTMLProps<HTMLAttributes<DshWebviewElement>, DshWebviewElement> & {
        src?: string
        partition?: string
        allowpopups?: string
      }
    }
  }
}
