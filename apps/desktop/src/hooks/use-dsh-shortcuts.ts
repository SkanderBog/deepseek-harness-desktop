import { createGlobalState } from '@reause/core'

export interface DshShortcutRow {
  id: string
  label: string
  keys: readonly string[]
  aria?: string
  available?: boolean
}

export const DSH_VIEW_COMMANDS = [
  { action: 'desktop-toggle-sidebar', command: 'sidebar.left.toggle', label: 'menu.toggle_sidebar' },
  { action: 'desktop-toggle-right-panel', command: 'sidebar.right.toggle', label: 'menu.toggle_right_panel' },
  { action: 'desktop-open-terminal', command: 'terminal.new', label: 'menu.open_terminal' },
  { action: 'desktop-search-chats', command: 'session.search', label: 'menu.search_chats' },
] as const

export type DshViewCommand = typeof DSH_VIEW_COMMANDS[number]['command']

export const useDshShortcuts = createGlobalState<{ rows: readonly DshShortcutRow[] }>({ rows: [] })

export function shortcutHint(rows: readonly DshShortcutRow[], id: string): string | undefined {
  const keys = rows.find(row => row.id === id)?.keys
  const text = keys?.join(' ')
  return text === undefined || text.length === 0 ? undefined : text
}
