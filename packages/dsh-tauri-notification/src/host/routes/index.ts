import { defineRoutes } from 'dsh-tauri'
import turnEnd from './turn-end/get'

/** 宿主路由：客户端在弹「回合完成」通知前先问一次结束原因。 */
export const routes = defineRoutes((disposer) => {
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-notification/turn-end' }, turnEnd)
})
