export interface ContentRiskFailureLike {
  code?: unknown
  message?: unknown
  status?: unknown
}

interface ContentRiskEventLike {
  type?: unknown
  seq?: unknown
  data?: {
    id?: unknown
    turn?: unknown
    inserted?: readonly { id?: unknown }[]
    reason?: { kind?: unknown, error?: unknown }
  }
}

export function isContentRiskFailure(value: unknown): boolean {
  if (typeof value !== 'object' || value === null)
    return false
  const failure = value as ContentRiskFailureLike
  if (failure.code === 'CONTENT_REJECTED')
    return true
  if (failure.status !== undefined && failure.status !== 400)
    return false
  return failure.code === 'INVALID_REQUEST'
    && typeof failure.message === 'string'
    && failure.message.trim() === 'Content Exists Risk'
}

export function contentRiskRecoveryBoundary(
  entries: readonly unknown[] | undefined,
  options: { historyComplete?: boolean } = {},
): number | undefined {
  if (entries === undefined)
    return undefined
  const events = entries.map(eventOf)
  let turnEndIndex = -1
  for (let index = events.length - 1; index >= 0; index -= 1) {
    if (events[index]?.type === 'turn/end') {
      turnEndIndex = index
      break
    }
  }
  const turnEnd = events[turnEndIndex]
  if (turnEnd === undefined || !isContentRiskFailure(turnEnd.data?.reason?.error))
    return undefined
  let lastSuccessfulEndIndex = -1
  for (let index = turnEndIndex - 1; index >= 0; index -= 1) {
    const event = events[index]
    const kind = event?.type === 'turn/end' ? event.data?.reason?.kind : undefined
    if (kind === 'completed' || kind === 'max-tokens') {
      lastSuccessfulEndIndex = index
      break
    }
  }
  if (lastSuccessfulEndIndex < 0 && options.historyComplete === false)
    return undefined

  const claimedIds = new Set<unknown>()
  for (let index = lastSuccessfulEndIndex + 1; index < turnEndIndex; index += 1) {
    const event = events[index]
    if (event?.type === 'user/message' && event.data?.id !== undefined)
      claimedIds.add(event.data.id)
  }
  if (claimedIds.size === 0)
    return undefined

  let insertionSeq: number | undefined
  for (let index = 0; index < turnEndIndex; index += 1) {
    const event = events[index]
    if (event?.type !== 'agent/inbox/spliced' || !event.data?.inserted?.some(message => claimedIds.has(message.id)))
      continue
    if (typeof event.seq === 'number' && Number.isSafeInteger(event.seq) && (insertionSeq === undefined || event.seq < insertionSeq))
      insertionSeq = event.seq
  }
  if (insertionSeq === undefined || insertionSeq <= 0)
    return undefined
  const boundary = insertionSeq - 1
  return events.some(event => event?.seq === boundary) ? boundary : undefined
}

function eventOf(value: unknown): ContentRiskEventLike | undefined {
  if (typeof value !== 'object' || value === null)
    return undefined
  const record = value as { event?: unknown }
  const event = record.event ?? value
  return typeof event === 'object' && event !== null ? event as ContentRiskEventLike : undefined
}
