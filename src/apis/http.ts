import { fetch as tauriFetch } from '@tauri-apps/plugin-http'
import { harness } from '@/store/modules/harness'

export interface FetchOptions {
  baseURL?: string
  method?: string
  params?: object
  body?: unknown
}

/** 本地实例答了，但没有这个 API：插件未加载（404 / 405），与网络不可达区分。 */
export class SshApiHttpError extends Error {
  readonly status: number

  constructor(status: number) {
    super(`SSH_API_HTTP_${status}`)
    this.status = status
  }
}

const MISSING_API_STATUSES = new Set([404, 405])

/** 请求走 Rust 侧（`@tauri-apps/plugin-http`）：内核发请求，没有 Origin，跨源限制不适用。 */
export async function ofetch<T>(path: string, options: FetchOptions = {}): Promise<T> {
  const response = await tauriFetch(urlOf(path, options), initOf(options))
  const payload = payloadOf(await response.text())
  if (!response.ok) {
    throw MISSING_API_STATUSES.has(response.status)
      ? new SshApiHttpError(response.status)
      : new Error(errorTextOf(payload, response.status))
  }
  return payload as T
}

function urlOf(path: string, options: FetchOptions): string {
  const query = new URLSearchParams()
  for (const [key, value] of Object.entries(options.params ?? {})) {
    if (value !== undefined)
      query.set(key, String(value))
  }
  const search = query.toString()
  return `${harness.$state.serviceUrl}${options.baseURL ?? ''}${path}${search === '' ? '' : `?${search}`}`
}

function initOf(options: FetchOptions): RequestInit {
  const init: RequestInit = { method: (options.method ?? 'get').toUpperCase() }
  if (options.body !== undefined) {
    init.body = JSON.stringify(options.body)
    init.headers = { 'content-type': 'application/json' }
  }
  return init
}

function payloadOf(text: string): unknown {
  if (text === '')
    return undefined
  try {
    return JSON.parse(text) as unknown
  }
  catch {
    return undefined
  }
}

function errorTextOf(payload: unknown, status: number): string {
  if (typeof payload === 'object' && payload !== null) {
    const value = payload as Record<string, unknown>
    if (typeof value.error === 'string')
      return value.error
    if (typeof value.message === 'string')
      return value.message
  }
  return `SSH_API_HTTP_${status}`
}
