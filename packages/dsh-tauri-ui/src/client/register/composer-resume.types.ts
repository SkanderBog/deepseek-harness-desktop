export interface ComposerListProjection {
  subscribe: (listener: () => void) => () => void
  getSnapshot: () => { current?: string }
}

export interface ComposerSessionSnapshot {
  running?: boolean
  removed?: boolean
  subagent?: unknown
}

export interface ComposerTurnEndReason {
  kind?: string
  error?: {
    code?: unknown
    message?: unknown
    status?: unknown
  }
}

export interface ComposerSessionEvent {
  type?: string
  seq?: number
  data?: {
    id?: unknown
    turn?: unknown
    inserted?: readonly { id?: unknown }[]
    reason?: ComposerTurnEndReason
  }
}

export interface ComposerSessionEventEntry {
  type?: string
  event?: ComposerSessionEvent
}

export interface ComposerSessionEventSource {
  subscribe: (listener: () => void) => () => void
  getSnapshot: () => { entries?: readonly ComposerSessionEventEntry[], hasMore?: boolean }
}

export interface ComposerSession {
  subscribe?: (listener: () => void) => () => void
  getSnapshot?: () => ComposerSessionSnapshot
  loadOlder?: () => Promise<void>
}

export interface ComposerSessionBinding {
  session?: ComposerSession
  eventSource?: ComposerSessionEventSource
}

export interface ComposerSessionsRuntime {
  list?: ComposerListProjection
  binding?: (sessionId: string) => unknown
  fork?: (options: { sessionId: string, atSeq: number, increaseTitle: boolean }) => Promise<string>
}

export interface ComposerIconState {
  path: string | null
  ariaLabel: string | null
  title: string | null
}
