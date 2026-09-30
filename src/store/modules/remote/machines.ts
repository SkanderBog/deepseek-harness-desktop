import type { SshConnectionState, SshEventEntry, SshMachineRow, SshProgress } from './types'

const STATES: SshConnectionState[] = ['disconnected', 'testing', 'connecting', 'connected', 'reconnecting', 'given-up']

const PHASES: SshProgress['phase'][] = ['handshake', 'installing', 'starting', 'probing', 'syncing']

/**
 * `GET /machines` 的壳层投影：丢坏行、合并手动机器与 ~/.ssh/config 别名，
 * 按名排序（localeCompare 的 ICU 整序在不同环境不稳定，故用码点比较）。
 */
export function machineRowsOf(value?: {
  enabled?: boolean
  items?: readonly unknown[]
  discovered?: readonly unknown[]
}): { enabled: boolean, machines: SshMachineRow[] } {
  const machines = [...(value?.items ?? []), ...(value?.discovered ?? [])]
    .flatMap((raw) => {
      const row = machineRowOf(raw)
      return row === undefined ? [] : [row]
    })
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
  return { enabled: value?.enabled === true, machines }
}

/** `GET /machines/events` 的壳层投影：seq/line 齐备才收。 */
export function eventEntriesOf(value?: { items?: readonly unknown[] }): SshEventEntry[] {
  return (value?.items ?? []).flatMap((raw) => {
    if (typeof raw !== 'object' || raw === null)
      return []
    const item = raw as Record<string, unknown>
    if (typeof item.seq !== 'number' || typeof item.line !== 'string')
      return []
    return [{ seq: item.seq, line: item.line }]
  })
}

function machineRowOf(raw: unknown): SshMachineRow | undefined {
  if (typeof raw !== 'object' || raw === null)
    return undefined
  const value = raw as Record<string, unknown>
  if (typeof value.id !== 'string' || value.id === '')
    return undefined
  if (typeof value.name !== 'string' || value.name === '')
    return undefined
  if (!STATES.includes(value.state as SshConnectionState))
    return undefined
  return {
    id: value.id,
    name: value.name,
    ...typeof value.color === 'string' && value.color !== '' ? { color: value.color } : {},
    ...value.tintBorder === true ? { tintBorder: true } : {},
    ...typeof value.host === 'string' && value.host !== '' ? { host: value.host } : {},
    ...typeof value.port === 'number' ? { port: value.port } : {},
    ...typeof value.user === 'string' && value.user !== '' ? { user: value.user } : {},
    ...typeof value.remotePort === 'number' ? { remotePort: value.remotePort } : {},
    ...typeof value.startCommand === 'string' && value.startCommand !== '' ? { startCommand: value.startCommand } : {},
    ...value.hasPassword === true ? { hasPassword: true } : {},
    ...value.hasPassphrase === true ? { hasPassphrase: true } : {},
    state: value.state as SshConnectionState,
    ...typeof value.tunnelBaseUrl === 'string' ? { tunnelBaseUrl: value.tunnelBaseUrl } : {},
    ...typeof value.lastError === 'string' ? { lastError: value.lastError } : {},
    ...typeof value.nextRetryAt === 'number' ? { nextRetryAt: value.nextRetryAt } : {},
    ...value.authMethod === 'agent' || value.authMethod === 'key' || value.authMethod === 'password'
      ? { authMethod: value.authMethod }
      : {},
    ...progressOf(value.progress),
  }
}

function progressOf(raw: unknown): { progress?: SshProgress } {
  if (typeof raw !== 'object' || raw === null)
    return {}
  const value = raw as Record<string, unknown>
  if (!PHASES.includes(value.phase as SshProgress['phase']))
    return {}
  return {
    progress: {
      phase: value.phase as SshProgress['phase'],
      ...typeof value.attempt === 'number' ? { attempt: value.attempt } : {},
      ...typeof value.total === 'number' ? { total: value.total } : {},
      ...typeof value.log === 'string' ? { log: value.log } : {},
    },
  }
}
