import type { NavbarProps } from './navbar'
import type { WebviewOutboundMessage } from './webview-bridge'

export const REMOTE_SECTION = 'dsh-tauri-remote'

export const REMOTE_TAB_MACHINES = 'machines'

export const REMOTE_TAB_SYNC = 'sync'

export const REMOTE_TAB_ACCESS = 'access'

export function navBridgeOf(post: (message: WebviewOutboundMessage) => void, live: boolean): Partial<NavbarProps> {
  if (!live)
    return {}
  return {
    onToggleSidebar: () => post({ type: 'dsh://sidebar:toggle' }),
    onNewChat: () => post({ type: 'dsh://session:new' }),
    onOpenFolder: () => post({ type: 'dsh://workspace:add' }),
    onOpenMachineManager: () => post({ type: 'dsh://settings:open', section: REMOTE_SECTION, tab: REMOTE_TAB_MACHINES }),
    onOpenSyncToRemote: () => post({ type: 'dsh://settings:open', section: REMOTE_SECTION, tab: REMOTE_TAB_SYNC }),
  }
}
