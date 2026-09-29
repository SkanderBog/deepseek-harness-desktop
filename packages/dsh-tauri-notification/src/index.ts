import { PLUGIN_ID } from './shared/constants'

export const name = PLUGIN_ID

/**
 * 宿主半边保持惰性。
 *
 * 本插件需要的通知数据（会话标题、待处理的授权/提问）只存在于客户端服务里，
 * 宿主侧既没有可注册的会话投影（`@deepseek-ai/dsh-session-projection` 不在本仓库
 * 依赖内），也没有需要暴露的 HTTP 路由，因此不订阅任何宿主事件、不注册任何服务。
 */
export function apply(): void {}
