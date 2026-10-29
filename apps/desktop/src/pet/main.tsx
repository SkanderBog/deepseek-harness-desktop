import { fetch as tauriFetch } from '@tauri-apps/plugin-http'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './app'
import { reportPetIssue } from './utils/log'
import './main.css'

const nativeFetch = globalThis.fetch
const pluginFetch = tauriFetch as unknown as typeof globalThis.fetch
globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
  const url = typeof input === 'string'
    ? input
    : input instanceof URL
      ? input.href
      : input.url
  return url.startsWith('https://')
    ? pluginFetch(input, init)
    : nativeFetch(input, init)
}) as typeof globalThis.fetch

globalThis.addEventListener('error', (event) => {
  reportPetIssue('window error', `${event.message} @ ${event.filename}:${event.lineno}`)
})
globalThis.addEventListener('unhandledrejection', (event) => {
  reportPetIssue('unhandled rejection', (event as PromiseRejectionEvent).reason)
})

const root = document.getElementById('root') as HTMLElement
root.className = 'h-full w-full'

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
