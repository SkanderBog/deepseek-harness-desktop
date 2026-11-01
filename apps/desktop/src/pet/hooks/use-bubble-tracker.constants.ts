import type { Motion } from 'dsh-pet-component'

export const STATUS_COPY: Record<string, number> = {
  thinking: 3,
  working: 3,
  result: 3,
  waiting: 3,
  approval: 3,
  success: 3,
  running: 2,
  review: 2,
  failed: 2,
  error: 3,
  stopped: 2,
}

export const ACTIVITY_COPY = new Set(['searching', 'editing', 'testing', 'commanding'])

export const DONE_BUBBLE_TIMEOUT = 3000
export const TERMINAL_MOTIONS = new Set<Motion>(['failed', 'review', 'error', 'success'])
export const WORK_STATUSES = new Set(['thinking', 'working', 'result', 'waiting', 'success', 'error'])
export const ACTIVE_MOTIONS = new Set<Motion>(['running', 'thinking', 'working', 'result'])

export const TOOL_LABELS: Record<string, string> = {
  pwsh: 'Pwsh',
  bash: 'Bash',
  grep: 'Grep',
  glob: 'Glob',
}

export const TRANSLATED_TOOLS = new Set(['read', 'read_image', 'write', 'edit', 'str_replace_editor', 'web_search', 'web_fetch', 'think', 'skill'])

export const TOOL_ARG_KEYS: Record<string, readonly string[]> = {
  pwsh: ['command'],
  bash: ['command'],
  grep: ['pattern'],
  glob: ['pattern'],
  read: ['file_path', 'path'],
  read_image: ['file_path', 'path'],
  write: ['file_path', 'path'],
  edit: ['file_path', 'path'],
  str_replace_editor: ['file_path', 'path'],
  web_search: ['queries', 'query'],
  web_fetch: ['url'],
  think: ['thought'],
  skill: ['name'],
}

export const TOOL_PATTERNS: Array<[RegExp, string]> = [
  [/search|grep|find|glob|web|read|fetch|open/, 'searching'],
  [/write|edit|patch|replace|create|move|delete/, 'editing'],
  [/test|check|lint|build|verify/, 'testing'],
  [/shell|bash|exec|command|terminal|powershell|pwsh/, 'commanding'],
]
