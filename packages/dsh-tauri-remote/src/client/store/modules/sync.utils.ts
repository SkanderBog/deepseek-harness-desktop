import type { SyncItemResult } from '../../types/index'
import type { SyncTarget } from './sync.types'

export function sameSyncTarget(a: SyncTarget | null | undefined, b: SyncTarget | null | undefined): boolean {
  return a != null && b != null
    && a.id === b.id && a.name === b.name && a.host === b.host
    && a.port === b.port && a.user === b.user && a.profileName === b.profileName
}

export function syncKeyOf(item: { kind: string, name: string, root?: string }): string {
  return `${item.kind}:${item.root ?? ''}:${item.name}`
}

export function toggleSelection(selected: ReadonlySet<string>, key: string): Set<string> {
  const next = new Set(selected)
  if (next.has(key))
    next.delete(key)
  else
    next.add(key)
  return next
}

export function mergeSyncResults(previous: readonly SyncItemResult[], incoming: readonly SyncItemResult[]): SyncItemResult[] {
  const merged = [...previous]
  const at = new Map(merged.map((item, index) => [syncKeyOf(item), index]))
  for (const item of incoming) {
    const key = syncKeyOf(item)
    const index = at.get(key) ?? merged.length
    at.set(key, index)
    merged[index] = item
  }
  return merged
}
