/**
 * 跨模块共享的类型。
 *
 * 这里只声明**结构类型**：`sessions` / `uiSession` 等服务由 dsh-client-ui 系列包提供，
 * 而它们不在本仓库的依赖内（只有宿主半边 @deepseek-ai 包可被本地解析），因此按
 * `source/dsh-notification` 的既有做法用最小结构面（structural face）读取，
 * 不 `import type` 那些缺席的包。
 */

/** `ObservableSnapshot<T>`：getSnapshot + subscribe 的最小面。 */
export interface ObservableSnapshotFace<T> {
  getSnapshot: () => T
  subscribe: (listener: () => void) => () => void
}

/** 会话摘要（`SessionListState.byId` 的值）。 */
export interface SessionSummaryFace {
  readonly id: string
  readonly title?: string
  readonly displayTitle?: string
  readonly running: boolean
  readonly updatedAt?: number
}

/** `SessionListState`：只取通知需要的字段。 */
export interface SessionListStateFace {
  readonly ids: readonly string[]
  readonly byId: Readonly<Record<string, SessionSummaryFace>>
}

/** 客户端 `sessions` 服务的最小面。 */
export interface SessionsFace {
  readonly list: ObservableSnapshotFace<SessionListStateFace>
}

/** 待处理交互的选项（`PendingQuestion` 的 questions 项）。 */
export interface PendingQuestionFace {
  readonly question?: string
  readonly header?: string
}

/** 授权结果：与官方 `ApprovalDecision` 对齐，本插件只会用 `allowed-once`。 */
export type ApprovalDecision = 'allowed-once' | 'rejected'

/**
 * 待处理交互（`PendingApproval` / `PendingQuestion`）。
 *
 * `kind` 取 `'approval' | 'question' | 'plan-review'`；`answerable` 与 `answer`
 * 只有已注册的交互才存在，因此都声明为可选。
 */
export interface PendingInteractionFace {
  readonly key: string
  readonly kind: string
  readonly sessionId: string
  readonly toolName?: string
  readonly callId?: string
  readonly reason?: string
  readonly displayReason?: unknown
  readonly answerable?: boolean
  readonly questions?: readonly PendingQuestionFace[]
  answer?: (outcome: ApprovalDecision) => Promise<void>
}

/** 单个会话的状态（`SessionStatus`）。 */
export interface SessionStatusFace {
  readonly running: boolean | undefined
  readonly pendingInteraction: PendingInteractionFace | undefined
  readonly completionUnread: boolean
}

/** 客户端 `uiSession` 服务的最小面。 */
export interface UiSessionFace {
  readonly sessionStatus: ObservableSnapshotFace<ReadonlyMap<string, SessionStatusFace>>
}

/** 原生通知按钮。 */
export interface NativeNotificationAction {
  readonly id: string
  readonly title: string
}

/** 交给宿主窗口补丁（`NOTIFICATION_SHIM_JS`）的原生通知描述。 */
export interface NativeNotificationInput {
  readonly title: string
  readonly body: string
  readonly tag: string
  readonly sessionId: string
  readonly requireInteraction?: boolean
  readonly actions?: readonly NativeNotificationAction[]
  /** 用户点通知本体（非按钮）。 */
  readonly onClick?: () => void
  /** 用户点按钮：回传 action id。 */
  readonly onAction?: (action: string) => void
}

/** 通知类别：决定文案与是否需要常驻。 */
export type NotificationKind = 'turn' | 'approval' | 'question'

/** 设置项：轮次完成通知时机。 */
export type TurnCompleteMode = 'never' | 'background' | 'always'

/** 设置项：通知提示音。 */
export type NotificationSound = 'default' | 'classic' | 'none' | 'custom'

/** 持久化设置。 */
export interface NotificationSettings {
  turnComplete: TurnCompleteMode
  approval: boolean
  question: boolean
  sound: NotificationSound
  customSound: string | null
}

/** store 状态 = 持久化设置 + 一次性水合标记。 */
export interface NotificationSettingsState extends NotificationSettings {
  hydrated: boolean
}
