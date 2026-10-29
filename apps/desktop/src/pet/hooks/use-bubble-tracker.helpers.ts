import type { Motion } from 'dsh-pet-component'
import { petI18n } from '../i18n'
import {
  ACTIVE_MOTIONS,
  ACTIVITY_COPY,
  STATUS_COPY,
  TOOL_ARG_KEYS,
  TOOL_LABELS,
  TOOL_PATTERNS,
  TRANSLATED_TOOLS,
  WORK_STATUSES,
} from './use-bubble-tracker.constants'

export interface SessionTitleSource {
  [key: string]: unknown
  title?: unknown
  displayTitle?: unknown
  name?: unknown
  phase?: unknown
  origin?: unknown
}

export interface BubbleSession {
  [key: string]: unknown
  id: string
}

export function seedNumber(seed: string | number | undefined): number {
  const value = String(seed ?? '')
  const numeric = Number(value)
  if (Number.isFinite(numeric))
    return Math.abs(Math.trunc(numeric))
  return [...value].reduce((total, char) => total + (char.codePointAt(0) ?? 0), 0)
}

export function statusCopy(group: string, seed?: string | number): string {
  const key = Object.hasOwn(STATUS_COPY, group) ? group : 'working'
  return petI18n.t(`pet.bubble.status.${key}.${seedNumber(seed) % STATUS_COPY[key]}`)
}

export function activityCopy(activity: string, seed?: string | number): string {
  return ACTIVITY_COPY.has(activity)
    ? petI18n.t(`pet.bubble.activity.${activity}.${seedNumber(seed) % 3}`)
    : statusCopy('working', seed)
}

export function taskCopy(task: string | undefined): string | undefined {
  const value = String(task ?? '').trim().replace(/[。！？.!?]+$/u, '')
  if (!value)
    return undefined
  if (petI18n.language !== 'zh-CN')
    return petI18n.t('pet.bubble.task', { task: value })
  if (/^(?:正在|继续)/u.test(value))
    return value
  if (/^(?:准备|检查|验证|修改|修复|测试|构建|整理|分析|梳理|查找|搜索|读取|实现)/u.test(value))
    return `正在${value}`
  return petI18n.t('pet.bubble.task', { task: value })
}

export function toolActivityGroup(tool: string | undefined): string {
  const value = String(tool || '').toLowerCase()
  return TOOL_PATTERNS.find(([pattern]) => pattern.test(value))?.[1] ?? 'working'
}

export function sessionTitle(session: SessionTitleSource): string {
  const base = [session.title, session.displayTitle, session.name]
    .find((v): v is string => typeof v === 'string' && v.trim().length > 0)
    ?.trim() ?? petI18n.t('pet.bubble.untitled')

  if (session.phase === 'approval')
    return `${petI18n.t('pet.bubble.approval')} · ${base}`
  if (session.phase === 'user-question' || session.phase === 'blocked')
    return `${petI18n.t('pet.bubble.choice')} · ${base}`
  if (session.origin === 'subagent')
    return `${petI18n.t('pet.bubble.subagent')} · ${base}`
  return base
}

export function rawSession(payload: unknown): BubbleSession | undefined {
  if (!payload || typeof payload !== 'object')
    return undefined
  const record = payload as Record<string, unknown>
  const session = (record.session && typeof record.session === 'object' ? record.session : record) as Record<string, unknown>
  const id = session.id ?? session.sessionId
  return typeof id === 'string' && id.length > 0 ? { ...session, id } : undefined
}

export function sessionMotion(session: BubbleSession): Motion | undefined {
  const work = session.workStatus as string
  if (WORK_STATUSES.has(work))
    return work as Motion

  const value = session.status ?? session.activity ?? session.phase
  const agentError = session.lastAgentError

  if (session.running !== true && (value === 'failed' || value === 'error' || (Boolean(agentError) && agentError !== 'aborted'))) {
    return 'failed'
  }
  if (value === 'review' || value === 'reviewing' || value === 'plan-review')
    return 'review'

  const hasPending = Array.isArray(session.pending) ? session.pending.length > 0 : Boolean(session.pending)
  if (value === 'waiting' || value === 'pending' || value === 'blocked' || Boolean(session.pendingInteraction) || hasPending) {
    return 'waiting'
  }
  if (value === 'running' || value === 'working' || value === 'thinking' || session.running === true) {
    return 'running'
  }

  return undefined
}

function sanitizeText(str: string): string {
  return str.replace(/\s+/g, ' ').trim()
}

function toolArgDetail(tool: string, args: unknown): string | undefined {
  if (typeof args !== 'string' || !args)
    return undefined
  try {
    const record = JSON.parse(args)
    if (!record || typeof record !== 'object' || Array.isArray(record))
      return undefined

    const keys = TOOL_ARG_KEYS[tool] ?? ['file_path', 'path']
    for (const key of keys) {
      const val = record[key]
      if (typeof val === 'string' && val.trim())
        return sanitizeText(val)
      if (Array.isArray(val)) {
        const first = val.find(item => typeof item === 'string' && item.trim())
        if (first)
          return sanitizeText(first)
      }
    }
  }
  catch {
  }
  return undefined
}

export function getLiveActivity(session: BubbleSession, motion: Motion): string | undefined {
  if (!ACTIVE_MOTIONS.has(motion) || !session.liveActivity || typeof session.liveActivity !== 'object') {
    return undefined
  }
  const { kind, text, name, args } = session.liveActivity as Record<string, unknown>

  if (kind === 'reasoning' && typeof text === 'string' && text.trim()) {
    return `${petI18n.t('pet.bubble.thought')} · ${sanitizeText(text)}`
  }
  if (kind === 'tool' && typeof name === 'string' && name) {
    const tool = name.toLowerCase()
    const label = TRANSLATED_TOOLS.has(tool) ? petI18n.t(`pet.bubble.tool.${tool}`) : TOOL_LABELS[tool]
    if (!label)
      return `${petI18n.t('pet.bubble.tool')} · ${name}`
    const detail = toolArgDetail(tool, args)
    return detail ? `${label} · ${detail}` : label
  }
  return undefined
}

export function bubbleContent(session: BubbleSession, motion: Motion) {
  const title = sessionTitle(session)
  const seed = session.id

  const fallbackCopy = motion === 'waiting'
    ? statusCopy(session.phase === 'approval' ? 'approval' : 'waiting', seed)
    : motion === 'working'
      ? activityCopy(toolActivityGroup(String((session.liveActivity as Record<string, unknown> | undefined)?.name ?? '')), seed)
      : statusCopy(motion, seed)

  const candidates = [
    session.lastAgentError ? petI18n.t('pet.bubble.failed', { error: String(session.lastAgentError) }) : undefined,
    taskCopy(session.task as string | undefined),
    getLiveActivity(session, motion),
    fallbackCopy,
    session.description,
    session.message,
  ]

  const description = candidates
    .find((item): item is string => typeof item === 'string' && item.trim().length > 0)
    ?.trim() ?? petI18n.t('pet.bubble.untitled')

  return {
    title,
    description,
    loading: ACTIVE_MOTIONS.has(motion),
  }
}
