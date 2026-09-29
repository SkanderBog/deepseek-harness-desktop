import { cssr, styles as sharedStyles } from 'dsh-tauri-ui/client'
import { SESSION_ICON_ATTRIBUTE } from '../constants'

const { c } = cssr
const { secondary } = sharedStyles

/**
 * 全局样式：只压官方会话树（`[role="treeitem"]`）——图标座位由 DOM 观察器注入，
 * 侧栏导航按钮里那个未读点也要靠 `:has()` 反过来定位宿主按钮。组件自身样式一律写在
 * 组件的 `className` 上（见 `components/scheduler.classes.ts` 与各组件）。
 */
export default c([
  c(`[${SESSION_ICON_ATTRIBUTE}]`, { width: '16px', height: '20px', flex: 'none', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', marginLeft: '2px', marginRight: '5px', color: secondary }),
  c('[role="treeitem"]', { position: 'relative' }),
  c(`[role="treeitem"]:hover [${SESSION_ICON_ATTRIBUTE}]`, { visibility: 'hidden' }),
  c('button:has(.dshp-scheduler__nav-dot)', { position: 'relative' }),
])
