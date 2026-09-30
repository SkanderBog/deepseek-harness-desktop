根据您提供的规范文档，以下是遵循您的四项核心优化准则（性能优化、结构简化、可读性增强、逻辑等价性保障）后输出的**代码/文档优化方案**。

---

## 优化后文档

# 壳插件安装管理器 `use-plugins-manager` 规范

> **状态**：待实现（TODO）
> **需求来源**：用户需求「壳插件安装重构：新增 use-plugins-manager」（接口详见 §三）
> **上游规范**：[devlopment.md](https://www.google.com/search?q=../specs/devlopment.md)（壳层通用）→ [desktop.baisc.md](https://www.google.com/search?q=../specs/desktop.baisc.md)（运行环境）→ [testing.md](https://www.google.com/search?q=../specs/testing.md)（测试质量）
> **冲突裁决**：`plugin.client.md` / `plugin.client.panel.md` / `plugin.host.service.md` 仅约束 `packages/*/src/{client,host}`，**不约束壳层 `src/**`；壳层 Store 允许 async action（既有先例：`preinstall`、`recovery` Store），但必须严格满足 SSOT（单一真实数据源）与「无模块级可变状态」。

---

## 一、目标与边界

### 1.1 核心目标

1. **统一收口**：壳层所有改变「已安装插件集合/启用状态」的操作（安装、升级、卸载、禁用、启用、授权），收口至管理器 Store 及唯一 React 门面 `useDshPluginsManager`。
2. **队列与执行语义**：
* **组间**：按独立 API 调用排队，**严格串行**执行。
* **组内**：一次 API 调用为一个「组」。安装/升级/卸载在组内合并为**一次宿主 CLI 调用（单次 pnpm）**；禁用/启用由于宿主无批量语义，组内并发提交并由宿主自行排队。
* **粒度**：每个插件在代码层面独立追踪进程状态、失败归因与授权状态。


3. **提示权收口**：管理器是唯一执行者与提示者（仅当 `toast: true` 时）；`toast: false`（如预装引导页）时，管理器仅暴露状态供调用方自行渲染。
4. **就地授权与续传**：DSH 精确版本豁免与 pnpm 发布保护期豁免在管理器内部就地完成；被拒插件不阻塞同组其他插件，授权或拒绝后将其余**剩余集合**重新提交（§五.6）。

### 1.2 边界声明（明确不做）

| 功能模块 | 现有实现落点 | 不并入管理器的理由 |
| --- | --- | --- |
| **快照与恢复** | `src/ui/config/plugin.tsx`, `src/ui/plugin/recovery.tsx` | 属于系统恢复能力，非安装/启用时序 |
| **内置插件自愈** | `src/store/modules/harness/store.ts` → Rust `internal/mod.rs` | 属于后台幂等自愈（`EnsureCoordinator`），无需用户交互队列 |
| **运行期错误上报** | `src/layout/components/iframe.tsx` | 仅涉及只读错误日志上报 |
| **远程机器插件同步** | `packages/dsh-tauri-ssh/src/host/service/plugins-sync.ts` | 走 Node/SSH Wire 传输，不经过 Tauri 命令 |
| **核心市场 UI** | 核心包 `@deepseek-ai/dsh-plugin-manager` | 仅复用其「安装前检查」逻辑，不接管其 UI 组件 |

> **范围补充**：**禁用/启用** 纳入本次重构。其作为插件加载状态迁移，与安装共享同一个队列、进程状态、授权暂停以及「组结算统一重启一次」语义（§八.5）。

---

## 二、现状问题与收敛对照

| # | 现状模式 | 存在缺陷 | 优化后机制 |
| --- | --- | --- | --- |
| **1** | `install_preinstall_plugins` 批量安装 | 单插件被拒导致整批失败且无法精细归因 | 保留「整批单次 pnpm」提高效率，失败时精细归因至具体进程，支持重新提交剩余集合 |
| **2** | 授权 UI 重复实现 | `setup-preinstall.tsx` 与 `plugin.tsx` 存在两套交互与文案 | 由管理器统一收口，集中处理 Toast 提示与授权响应 |
| **3** | 授权命令重复调用 | 两处不同模块分别调用后端同一对授权命令 | 收口至 `manager.approve` 统一处理 |
| **4** | Busy 状态分散 | `plugin.tsx` busy、`preinstall` 状态与 `recovery.busy` 互相独立 | 统一收口至管理器单一队列与 `PluginProcessStatus` 状态机 |
| **5** | 插件列表重复查询 | 3 处 UI 组件各自独立维护 `get_dsh_plugins` 逻辑 | 统一通过 `manager.installed` 进行集中数据投影 |
| **6** | 服务重启高频触发 | 各操作独立调用 `store.harness.restart()`，导致多次无谓重启与「装到一半服务没了」 | 插件操作不停服；组结算后统一评估并**询问一次**是否重启 |
| **7** | 工具函数反向依赖 | `parseBlockedRefusal` 挂在预装模块，导致面板反向依赖 | 迁移至 `src/store/modules/plugins/utils.ts` |
| **8** | 引导逻辑重复 | 两处入口分散处理「打开预设引导」 | 统一收口为 `preinstall.open()` |
| **9** | 面板缺乏安装入口 | 无法在面板直接输入 Spec 安装新插件 | 新增标准的插件安装与兼容性检查入口 |
| **10** | 禁用/启用路径孤立 | 禁用/启用操作独立 Reboot、独立 Busy | 并入管理器队列，统一调度与确认流 |

---

## 三、对外契约（TypeScript 类型与 API）

类型定义文件位于 `src/store/modules/plugins/types.ts`。

### 3.1 类型定义

```ts
/** 已安装插件视图（由宿主 DshPlugin 投影转化） */
export interface Plugin {
  id: string
  name: string
  version: string
  description: string
  repoUrl: string
  bundled: boolean
  disabled: boolean
  patchDisabled: boolean
  recommended: boolean
  fix: boolean
  internal: boolean
  hasSnapshot: boolean
  error: PluginErrorInfo | null
  /** 最新可升级版本；无更新探测缓存时为 null */
  latest: string | null
  updateAvailable: boolean
  /** 已装版本对当前核心的兼容性 */
  incompatible: boolean
  /** 最新版本对当前核心的兼容性 */
  latestIncompatible: boolean
}

/** 插件引用表达：纯 Spec 字符串，或结构化对象 */
export type PluginRef = string | { spec: string; version?: string }

export type PluginProcessType = 'install' | 'upgrade' | 'uninstall' | 'disable' | 'enable'

export type PluginProcessStatus = 'pending' | 'running' | 'unauthorized'

export interface PluginProcess {
  /** 同组内唯标识：`${groupId}#${index}` */
  id: string
  groupId: string
  type: PluginProcessType
  status: PluginProcessStatus
  /** 归一化后的提交 Spec */
  spec: string
  /** 展示名称 */
  name: string
  /** 显式指定的版本 */
  version?: string
  /** 被宿主拦截时的原始拒绝载荷 */
  refusal?: BlockedRefusal
  /** 进度 Toast 句柄 */
  progressKey?: string
  /** 授权 Toast 句柄 */
  approvalKey?: string
}

export interface PluginProcessResult {
  process: PluginProcess
  ok: boolean
  error?: string
  code?: string
  reason?: 'not-installed' | 'already-absent' | 'update-hold' | 'rejected' | 'cancelled' | 'retry-exhausted'
}

export interface PluginManagerLog {
  at: number
  level: 'info' | 'error'
  message: string
  groupId?: string
  processId?: string
}

export interface PluginSearchResult {
  spec: string
  name?: string
  version?: string
  compatible: boolean | null
  peers?: Record<string, string>
  problem?: 'invalid-spec' | 'not-found' | 'network' | 'unsupported' | 'unknown'
}

```

### 3.2 API 契约与使用规范

```ts
const manager = useDshPluginsManager({ toast: true, restartOnSettle: true })

```

#### 成员属性与方法列表

| 成员/方法 | 类型/签名 | 说明 |
| --- | --- | --- |
| `installed` | `Plugin[]` | 当前已安装插件的视图列表 |
| `processes` | `PluginProcess[]` | 当前正在运行或排队中的未终结进程 |
| `pendingApprovals` | `PluginProcess[]` | 等待用户授权的进程 (`status === 'unauthorized'`) |
| `logs` | `PluginManagerLog[]` | 全局日志时间线（最多保留 200 条） |
| `install` | `(refs: PluginRef | PluginRef[]) => Promise<PluginProcessResult[]>` | 提交安装组。传入数组时合并为一次 pnpm 命令 |
| `upgrade` | `(refs: PluginRef | PluginRef[]) => Promise<PluginProcessResult[]>` | 提交升级组 |
| `uninstall` | `(refs: PluginRef | PluginRef[]) => Promise<PluginProcessResult[]>` | 提交卸载组 |
| `disable` | `(refs: PluginRef | PluginRef[]) => Promise<PluginProcessResult[]>` | 提交禁用组（组内并发执行） |
| `enable` | `(refs, options?: { clearConfigOverride?: boolean }) => Promise<PluginProcessResult[]>` | 提交启用组 |
| `approve` | `(refs?: PluginRef | PluginRef[]) => Promise<PluginProcessResult[]>` | 授权并重试；若参数为空，则授权当前所有待授权进程 |
| `reject` | `(ref: PluginRef) => Promise<PluginProcessResult[]>` | 拒绝授权（等价于关闭授权弹窗） |
| `cancel` | `() => Promise<void>` | 取消当前活动组的执行 |
| `search` | `(refs: PluginRef | PluginRef[], options?: { dsh?: string }) => Promise<PluginSearchResult[]>` | 查询插件与当前核心的兼容性 |
| `on` | `(event: ManagerEvent, cb: Function) => () => void` | 订阅管理器事件（返回取消订阅回调） |

#### 通用准则

1. **统一返回数组**：写操作方法均返回 `Promise<PluginProcessResult[]>`。除了非法参数输入会直接 throw 外，内部执行失败均通过 `ok: false` 表达，**绝不抛出 Unhandled Rejection**。
2. **Resolve 时机**：返回的 Promise 在**整个任务组结算完成**（组内全部进程达到终态）后 Resolve。
3. **Ref 归一化规范**：
* `'aaa'` $\rightarrow$ `{ spec: 'aaa', name: 'aaa' }`
* `'aaa@^1.2.0'` $\rightarrow$ `{ spec: 'aaa@^1.2.0', name: 'aaa' }`
* `{ spec: 'aaa', version: '1.2.3' }` $\rightarrow$ `{ spec: 'aaa@1.2.3', name: 'aaa', version: '1.2.3' }`（`version` 覆盖 spec 内既有范围）
* Scoped 包名（如 `'@scope/name@1.0.0'`）：仅按**末尾** `@` 进行切分。


4. **事件总线**：基于 `createEventHook` 暴露 `completed`、`allcompleted`、`error`、`approve` 事件，提供轻量解耦的状态监听能力。

---

## 四、内部状态数据结构

核心逻辑位于 `src/store/modules/plugins/` 目录下。

```ts
interface PluginGroup {
  id: string
  type: PluginProcessType
  status: 'pending' | 'active' | 'settled'
  processIds: string[]
  /** 重试尝试次数，上限 8 次 */
  attempt: number
  /** 本次实际提交给宿主的 Spec/ID 集合 */
  pending: string[]
  /** 组结算最终结果 */
  results: PluginProcessResult[]
  resume?: () => void
  done: Promise<PluginProcessResult[]>
  resolveDone: (results: PluginProcessResult[]) => void
}

// Store State 接口
interface PluginsState {
  groups: PluginGroup[]
  processes: PluginProcess[]
  logs: PluginManagerLog[]
  activeGroupId: string | null
  cancelling: boolean
}

```

---

## 五、队列、并发与状态流转

### 5.1 队列调度机制

```
  [API 触发] ──► 归一化 Ref ──► 创建 PluginGroup ──► 入队 groups ──► 触发 drain()
                                                                       │
┌──────────────────────────────────────────────────────────────────────┘
▼
评估 activeGroupId:
 ├─► 若已有活动组 ──► 保持等待 (Pending)
 └─► 若无活动组   ──► 标记组为 Active ──► 组内提交宿主 ──► 状态流转 ──► 组结算 ──► 取下一组

```

### 5.2 组内批处理策略

```
                      ┌───► pnpm 方式: 合并为单次 CLI 命令提交 (install/upgrade/uninstall)
[组内进程集合] ───────┤
                      └───► 并发方式: Promise.allSettled 并发调用 (disable/enable)

```

* **批处理**：安装/升级/卸载直接将多个 Spec 拼接为一次 `dsh plugin add/update/remove` 调用，利用底层 pnpm 自身并行能力，不通过 `for await` 逐项串行。
* **并发处理**：禁用与启用由于宿主命令仅支持单 ID，前端通过 `Promise.allSettled` 并发分发，由宿主进程锁自动排队。

### 5.3 进程状态流转

```
                ┌─────────────────── (提交宿主) ───────────────────┐
                │                                                  │
                ▼                                                  │
   ┌────────────────────────┐                             ┌─────────────────┐
   │    status: pending     │                             │ status: running │
   └────────────────────────┘                             └────────┬────────┘
                ▲                                                  │
                │ (授权重试)                                       │ (宿主响应)
                │                                                  ▼
   ┌────────────────────────┐                             ┌─────────────────┐
   │  status: unauthorized  │◄─── (需要用户授权 / 拦截) ────┤   结果处理/归因  ├─► 成功/其他失败 ─► [终结并移出]
   └────────────┬───────────┘                             └─────────────────┘
                │
                └─── (拒绝 / 取消) ───────────────────────────────────────────────────► [终结并移出]

```

1. **提交阶段**：组发起时，组内所有进程同步由 `pending` 切换至 `running`。
2. **拦截归因**：
* 宿主返回授权拦截码（如 `PLUGIN_VERSION_INCOMPATIBLE`）时，通过 `parseBlockedRefusal` 匹配目标进程。
* 匹配成功的进程切换为 `unauthorized` 并挂载 `refusal` 载荷；同组其他未被拦截的进程**重置回 `pending**`。
* 隐藏当前组的加载 Toast，暂停队列推进，等待用户决策。
* 例外：升级返回 `PLUGIN_UPDATE_NO_CHANGE` 时按「有没有可安装的 registry 目标」分流，**不允许把可升级的插件判成「保持原样」**。载荷里带版本（面板此刻显示着目标版本，或探测缓存里有）说明这个目标必须装上：`retryable === true` 的先走授权流程（宿主按我们自己请求的目标合成 `PLUGIN_POLICY_BLOCKED`，前端照常挂 `unauthorized`）；`retryable === false`（该精确版本已授权过）由宿主的显式安装重跑兜底。两条路都没装上就是**真实失败**——`ok: false` 且 `error` 带上原因，结果提示用 `danger` 并把原因放进 `description`（用户抱怨过「没有任何授权和信息」）。只有载荷里根本没有候选版本的条目（git 提交 / 本地目录 / 探测不到的 registry 目标）才以 `ok: false, reason: 'update-hold'` 静默结算，且不计入汇总的失败数。
* 注意：**授权过的精确版本必须真的装得上**。pnpm 的发布时长豁免（`minimumReleaseAgeExclude`）只被解析阶段采信，lockfile 校验阶段照旧按默认窗口拦截，因此宿主在「本批可升级目标全部已授权」时追加 `--config.minimumReleaseAge=0`（见 §8.1）。缺了这一步，用户在界面上点了授权，重跑依旧只得到 `PLUGIN_UPDATE_NO_CHANGE`——「没有可授权的新版本」是假象。
* 注意：**把来源钉死的声明要由显式安装推平**。`catalog:` 条目、git ref、`link:` 本地目录、精确版本这些被 pnpm 在 `update --latest` 时原样保留（git spec 甚至不会被退化成 semver 范围），于是即使用户已经授权过那个精确版本，升级核验仍是「没有变化」。因此**面板知道目标版本的升级一律走确定性显式安装**：`PluginRef` 归一化成 `{ spec: id, version: plugin.latest }`，`update_dsh_plugins` 的条目变成 `id@version`，宿主直接 `dsh plugin add <id>@<version>`（不再靠 `--latest` 去猜），随后按该版本核验；只有没带版本的老式条目才退回 `update --latest` + `force_upgrade_spec` 兜底。`latest` 不是 registry 版本（git 提交 / 本地目录）时没有可安装的版本，按上一条静默结算。
* 注意：**lock 里的太新条目会让此后每一次插件操作都失败**。放宽窗口装上的那一次会把太新的版本写进 `pnpm-lock.yaml`，而豁免清单不在 lock 里，于是 lockfile 校验阶段每次都拦它（现象：面板里一堆「升级插件 X 失败」且没有任何授权信息，`单个升级` 却正常；启动期内置插件安装失败则直接报 `INTERNAL_PLUGIN_INSTALL_FAILED`）。宿主因此加一步**自愈**：失败输出里被拦的条目**全部已在 lock 中**（说明早就装到本机，不是本次要审的新版本）就补写豁免并放宽窗口重跑一次；只要有一个不在 lock 中（真·新版本）就保持默认窗口，交本节的授权流程处理。批量安装（§8.1 `install_plugin_specs`）与升级/卸载走同一判据。


3. **重新提交剩余集合**：
* 授权（`approve`）或拒绝（`reject`）后，剔除已终结或已授权项，提取**剩余待执行集合**，再次作为单次 CLI 命令重新提交。



### 5.4 组结算与服务重启

当组内全部进程达到终态（成功、失败或拒绝）后：

1. **询问是否重启（并进结果提示）**：结算**不**立即重启。若组内发生了实际变更（至少 1 项成功）且 `restartOnSettle === true`，则把「重启」按钮并进随后那条结果／汇总 Toast（`variant: 'accent'`、`timeout: 0`）——点按钮才执行**一次** `store.harness.restart()`，直接关闭即「暂不重启」。插件操作本身不停服，改动要等重启后生效。一次入队只留**一条**消息：单结果时结果 Toast 本身就是那条「已完成＋重启」，多结果时是队列汇总那条，都不再另弹常驻重启气泡。
2. **事件分发**：触发 `completed` 与 `error`（若存在失败项）事件。
3. **清理与推进**：移除当前 `PluginGroup`，重置 `activeGroupId` 为 `null`，并递归调用 `drain()` 推进下一队列组。若队列为空，触发 `allcompleted`。

---

## 六、授权与拒绝处理

### 6.1 授权流程

```
[收到 unauthorized 状态]
       │
       ├─► 渲染常驻 Toast (timeout: 0, 带有 "授权" 按钮)
       │
       ▼
[用户点击 "授权"]
       │
       ├─► 执行 manager.approve(process)
       ├─► 调用宿主 allow_* 接口写入豁免配置
       ├─► 进程状态恢复为 pending
       └─► 重新提交当前组的剩余集合 (attempt + 1)

```

### 6.2 拒绝流程

```
[用户点击关闭 Toast (x)]
       │
       ├─► 触发 Toast onClose (Reason: 'dismissed')
       ├─► 自动触发 manager.reject(process)
       ├─► 进程移出队列，标记状态为 ok: false, reason: 'rejected'
       └─► 若组内仍有剩余项 ──► 立即将剩余集合重新提交宿主

```

---

## 七、Toast UI 状态映射

管理器的 Toast 采用**声明式状态投影**，在入队时的 `toast: true` 生效（与「面板是否挂载」无关：关闭插件面板不该让仍在跑的队列失去进度气泡与授权入口）：

```
                              ┌─── n = 1 : 显示 "正在安装 {{name}}..."
                              │
┌─── 组正在运行 (n 个进程) ────┼─── n >= 2: 显示 "正在安装 {{count}} 个插件..."
│                             │
│                             └─── 存在 unauthorized: 隐蔽加载 Toast，避免界面干扰
│
├─── 进程授权等待 ─────────────► 为每个待授权项弹出独立的常驻 Toast (timeout: 0)
│
└─── 队列排空 ────────────────► 整条队列（一次入队的所有组）跑完后**只汇报一次**：
                                「{{name}} 升级完成」＋是否重启（失败时 `danger`
                                并把 `error` 放进 `description`）；
                                用户自己取消／拒绝授权的结果不补提示；
                                没有任何候选版本的 `update-hold` 也不出声；
                                多结果弹**一条**队列汇总「插件操作已完成」，描述为
                                「x 个成功 · x 个失败 · x 个无需变更」（非零项才出现）。
                                需要重启时「重启」按钮就挂在这条汇总（或单结果那条）
                                上（`timeout: 0`），不额外再弹常驻重启提示。
                                授权／拒绝等中途提示照旧即时出现，不受此推迟影响。

```

> ** Toast 基础类库扩展要求**：`src/utils/toast.ts` 的 `onClose` 回调必须扩展 `reason` 参数（`'closed'` | `'evicted'` | `'dismissed'`），仅当为 `'dismissed'`（用户主动关闭）时触发 `reject` 逻辑。

---

## 八、宿主侧接口改动 (Rust / Tauri Bridge)

### 8.1 接口变更明细表

| 命令 (Command) | 参数类型 | 返回类型 | 修改说明 |
| --- | --- | --- | --- |
| `install_plugin_specs` | `specs: Vec<String>` | `Result<(), String>` | **新增**：支持传入多个 Spec，合并为单次 `dsh plugin add` 执行 |
| `update_dsh_plugins` | `ids: Vec<String>` | `Result<(), String>` | **变更**：替代原 `update_dsh_plugin`，支持批量更新；每一项可以是 `<id>` 或 `<id>@<版本>`（参数名保留 `ids` 以免改动前端载荷键） |

> `update_dsh_plugins` 的两条路径：升级**不再靠 `--latest` 猜**。面板此刻显示着目标版本，因此前端传 `id@version`，宿主对这些条目直接 `dsh plugin add <id>@<version>`（一条 CLI 调用装一批，装完按该精确版本核验；「已经就是这个版本」也算成功）。没带版本的条目才退回 `dsh plugin update --latest`。发布时长门禁拦住显式安装时，宿主按**我们自己请求的目标**合成 `PLUGIN_POLICY_BLOCKED: [{"name","version"}]` 交给授权流程——pnpm 在解析阶段报的文本不含精确发布时间，`diagnose::policy_blocked_versions` 有意不认它，所以这份载荷必须由升级路径自己给出。用户授权后目标进入 `minimumReleaseAgeExclude`，重跑时该批**每个 registry 目标都已豁免**才追加 `--config.minimumReleaseAge=0`（放宽是全调用生效的，不能让未授权的包顺带越过窗口），于是授权过的精确版本必然装上。
| `remove_dsh_plugins` | `ids: Vec<String>` | `Result<(), String>` | **变更**：替代原 `remove_dsh_plugin`，支持批量移除 |
| `inspect_plugin_specs` | `specs: Vec<String>, dsh: Option<String>` | `Result<Vec<PluginInspect>, String>` | **新增**：只读检查 Spec 兼容性，不改动本地 Profile |
| `cancel_plugin_processes` | 无 | `Result<(), String>` | **重命名**：由 `cancel_preinstall_plugins` 重命名，提升为通用方法 |

### 8.2 核心兼容性模块 (`src-tauri/src/service/plugin/compat.rs`)

新增 Rust 模块，负责解析与校验 DSH 核心 Peer 依赖：

* 提取插件声明中的 `@deepseek-ai/dsh` 相关 peerDependencies。
* 使用 `semver` crate 执行匹配校验。若解析过程发生异常，遵循 **Fail-Open（默认视作兼容）** 原则，避免阻断正常流程。

---

## 九、架构落地路径

### 9.1 文件组织结构

```
src/
├── store/
│   └── modules/
│       └── plugins/
│           ├── index.ts        # 集中导出桶文件 (Barrel)
│           ├── store.ts        # Pinia/Store 逻辑实现
│           ├── types.ts        # 类型定义
│           ├── utils.ts        # Ref 归一化、解析与辅助纯函数
│           └── events.ts       # 基于 createEventHook 的事件定义
└── hooks/
    └── use-plugins-manager.ts  # React 封装门面 Hook

```

### 9.2 数据流向图

```
宿主事件 / React Query (queryKeys.plugins)
                   │
                   ▼
       DshPlugin 原生数据列表
                   │
                   ▼
      enrichInstalled() 数据转换
                   │
                   ▼
         manager.installed 视图

```

---

## 十、实施阶段规划

```
[P0 基座构建] ──► [P1 Store & Hook 实现] ──► [P2 配置面板收口] ──► [P3 新增安装入口] ──► [P4 预装引导页收口] ──► [P5 垃圾清理]

```

1. **P0 基座**：完成 Rust 命令改造、兼容性校验模块编写与 Toast 原因透传扩展。
2. **P1 管理器**：实现 `src/store/modules/plugins/` 状态机与 `useDshPluginsManager`。
3. **P2 面板收口**：重构 `plugin.tsx` 面板，使其全部切至管理器 API。
4. **P3 新增安装入口**：基于 `manager.search` 与 `manager.install` 构建面板安装交互 UI。
5. **P4 引导页收口**：改造预装页，配置 `useDshPluginsManager({ toast: false, restartOnSettle: false })` 模式。
6. **P5 清理**：移除废弃代码与冗余错误码。

---

## 十一、自动化测试覆盖要求

### 11.1 前端单元测试 (Vitest)

新建并完善以下测试用例：

1. `test/plugin-manager-queue.test.ts`
* 验证组间串行执行顺序。
* 验证 `install(['a', 'b'])` 是否严格触发**单次**宿主调用。
* 验证禁用/启用操作在组内的**并发**分发。


2. `test/plugin-manager-approve.test.ts`
* 验证捕获拒绝载荷后的归因定位，以及未受影响进程恢复为 `pending`。
* 验证 `approve` 后仅将剩余集合重新提交宿主。
* 验证关闭 Toast 触发 `reject` 后自动从剩余集合中剔除对应项。


3. `test/plugin-manager-settle.test.ts`
* 验证组结算时 `restartOnSettle` 对重启调用的触发逻辑。


4. `test/plugin-manager-api.test.ts`
* 验证 Ref 归一化纯函数的边界输入。
* 验证幂等性判断（如卸载未安装插件返回 `already-absent`）。
* 验证面板显示的目标版本以 `id@版本` 形式传给宿主（`{ ids: ['aaa@2.0.0'] }`）。


5. `test/plugin-manager-update-hold.test.ts`
* 验证批处理载荷的逐项归因：未被点名的成员按成功结算、被点名且可授权的进授权流程、已授权却仍未装上的按**真实失败**上报（`danger` + 原因），没有任何候选版本的条目静默结算（不出提示、不计失败）。


6. `test/plugin-manager-summary.test.ts`
* 验证一次入队多个组时只在队列排空后弹**一条**汇总（计数正确、失败时 `danger`、需要重启时按钮挂在这条上并只触发一次 `restart`）。



### 11.2 后端单元测试 (Cargo Test)

1. `src-tauri/src/service/plugin/compat.rs`
* 覆盖 Workspace 语法解析、预发布版本匹配、非法 Range 兼容性降级及合并 `dependencies` 与 `peerDependencies` 的断言测试。


2. `src-tauri/src/service/plugin/install/single.rs`
* 覆盖 `split_upgrade_spec`（scoped 包、`id@next`、git spec 的切分）、`update_failure_payload`（多条 hold 合并成一个数组、真失败优先）、`force_upgrade_spec`（只有已授权的 registry 目标才补显式安装）与 `policy_refusal_from_specs`（按请求目标合成拒绝载荷）。
* 覆盖 `install/mod.rs` 的 `locked_release_age_exemptions`（被拦条目必须全部已在 lock 中才自愈）。