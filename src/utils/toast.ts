import type { ToastContentValue } from '@heroui/react/toast'
import type { ToastVariants } from '@heroui/styles'
import type { ReactNode } from 'react'
import { ToastQueue } from '@heroui/react'
import { hooks } from '@/config/hooks'

/** toast 关闭来源：库侧 close 不携带原因，只能由本模块预记或按超时配置反推 */
export type ToastCloseReason = 'closed' | 'evicted' | 'dismissed'

/** toast() 可选项：库内未暴露的 HeroUIToastOptions（toast-queue 收敛的 content + 超时回调），这里用公开的 ToastContentValue 组合 */
export type ToastOptions = Partial<ToastContentValue & { timeout?: number, onClose?: (reason: ToastCloseReason) => void }> & { placement?: Placement }
export type ToastUpdateOptions = Partial<ToastContentValue>

export type Placement = NonNullable<ToastVariants['placement']>
export const placements = [
  'top start',
  'top',
  'top end',
  'bottom start',
  'bottom',
  'bottom end',
] as const

/**
 * 单个 placement 同时存在的 toast 上限：渲染层（maxVisibleToasts）与丢弃层
 * （placementKeys 超限关最旧）共用同一数值。
 */
const MAX_VISIBLE_TOASTS = 3

export const queues = Object.fromEntries(
  placements.map(p => [p, new ToastQueue({ maxVisibleToasts: MAX_VISIBLE_TOASTS })]),
) as Record<Placement, ToastQueue>

const linuxQueues = Object.fromEntries(
  placements.map(p => [p, new ToastQueue({ maxVisibleToasts: MAX_VISIBLE_TOASTS, wrapUpdate: fn => fn() })]),
) as Record<Placement, ToastQueue>

export const activeQueues = navigator.platform.toLowerCase().includes('linux')
  ? linuxQueues
  : queues

const toastContents = new Map<string, ToastContentValue>()
const placementsKeys = new Map<string, Placement>()
/**
 * 程序化关闭的原因旁路表。底层 react-stately 的 `close(key)` 对「自动超时 / 用户点
 * 关闭 / 外部 close」走同一条路径且不透传参数，本模块只能在调用 close 前先登记，
 * 回调时取出；未登记（用户点关闭按钮）再按超时配置反推，见 `takeCloseReason`。
 */
const closeReasons = new Map<string, ToastCloseReason>()
/**
 * 每个 placement 的存活 key（按创建顺序，旧→新）。stately queue 对超出
 * maxVisibleToasts 的条目只做「窗口外排队、等旧条目关闭后复现」，常驻
 * （timeout: 0）气泡会无限积压并在旧气泡关闭时复现；这里在新 toast 入队后
 * 直接关闭最旧的条目，保证任何时刻只存在最新的 MAX_VISIBLE_TOASTS 条。
 */
const placementOrder = new Map<Placement, string[]>()

/**
 * 同步清理一条 toast 的登记。`keepReason` 为真时保留关闭原因，供库侧稍后
 * （rAF 异步）触发的 onClose 读取——`close()` 必须这样调用，否则常驻气泡
 * （`timeout: 0`）的程序化关闭会被误判为用户拒绝。
 */
function forgetKey(key: string, keepReason = false): void {
  toastContents.delete(key)
  if (!keepReason)
    closeReasons.delete(key)
  const placement = placementsKeys.get(key)
  placementsKeys.delete(key)
  if (placement === undefined)
    return
  const order = placementOrder.get(placement)
  if (order === undefined)
    return
  const index = order.indexOf(key)
  if (index >= 0)
    order.splice(index, 1)
  if (order.length === 0)
    placementOrder.delete(placement)
}

/**
 * 关闭原因：优先取本模块登记的程序化原因；未登记说明是用户在气泡上点了关闭
 * （或自动超时），按该条是否配了自动关闭超时反推——配了就是超时自动消失，
 * 没配（`timeout: 0` 常驻）只能是用户点了关闭。
 */
function takeCloseReason(key: string, autoClose: boolean): ToastCloseReason {
  const recorded = closeReasons.get(key)
  if (recorded !== undefined) {
    closeReasons.delete(key)
    return recorded
  }
  return autoClose ? 'closed' : 'dismissed'
}

/**
 * 统一 toast API：直接调用创建，toast.update/close/clear 通过 key 管理。
 * update 触发 `hooks['toast.updated']` 事件（见 config/hooks），由 ToastProvider 消费后
 * 原地更新对应 queue 的 content（HeroUI ToastQueue 没有 update 方法）。
 */
export const toast = Object.assign(
  (message: string | ReactNode, options?: ToastOptions) => {
    // 默认右下角；个别调用方需要其他位置时显式传 placement
    const { placement = 'bottom end', timeout, onClose, ...rest } = options || {}
    const content = { title: message, ...rest }
    // 未指定 timeout 时 HeroUI 会补默认超时（`constants: DEFAULT_TOAST_TIMEOUT`），
    // 因此「undefined 或正数」都意味着这条会自己消失。
    const autoClose = timeout === undefined || timeout > 0
    const key = activeQueues[placement].add(content, {
      timeout,
      onClose: () => {
        // 自动超时 / 用户关闭 / 外部 close 都会走到这里；延后业务回调，避免渲染中更新组件。
        const reason = takeCloseReason(key, autoClose)
        forgetKey(key)
        queueMicrotask(() => onClose?.(reason))
      },
    })
    toastContents.set(key, content)
    placementsKeys.set(key, placement)
    const order = placementOrder.get(placement) ?? []
    order.push(key)
    placementOrder.set(placement, order)
    // 丢弃超出上限的最旧条目（含 rAF 清理延迟期内的死 key），维持「仅最新 N 条」
    while (order.length > MAX_VISIBLE_TOASTS) {
      const oldest = order.shift()
      if (oldest !== undefined) {
        closeReasons.set(oldest, 'evicted')
        activeQueues[placement].close(oldest)
      }
    }
    return key
  },
  {
    update(key: string, options: ToastUpdateOptions): void {
      if (!placementsKeys.has(key))
        return
      toastContents.set(key, { ...(toastContents.get(key) ?? {}), ...options })
      void hooks['toast.updated'].trigger({ key, options })
    },

    close(key: string): void {
      const placement = placementsKeys.get(key)
      if (placement) {
        closeReasons.set(key, 'closed')
        activeQueues[placement].close(key)
        // onClose 是 rAF 异步回调：保留关闭原因到那时再取，但同步清掉其余登记，
        // 紧随其后的 add 不会把死 key 计入限额
        forgetKey(key, true)
      }
      else {
        toastContents.delete(key)
        closeReasons.delete(key)
      }
    },

    clear(): void {
      toastContents.clear()
      placementsKeys.clear()
      placementOrder.clear()
      closeReasons.clear()
      placements.forEach(p => activeQueues[p].clear())
    },
  },
)
