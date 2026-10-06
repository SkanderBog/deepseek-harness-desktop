import type { ComposerSessionEventEntry, ComposerSessionSnapshot } from './composer-resume.types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { composerResumeFeature } from './composer-resume'

const ARROW_PATH = 'M8.3125 0.980183C8.66767 1.0531'
const PLAY_FILL_PATH = 'M14.642 6.285c1.294.777 1.294 2.653 0 3.43l-9.113 5.468c-1.333.8-3.028-.16-3.029-1.715V2.532C2.5.978 4.196.018 5.53.818z'

const RESUME_LABEL = 'resumeTask'
const CONTENT_RISK_RECOVERY_LABEL = 'recoverContentRisk'

interface ControllerStub {
  triggerObserve: () => void
  click: (event: unknown) => void
  dispose: () => void
}

const mocks = vi.hoisted(() => ({
  resumeComposer: vi.fn(async () => ({ ok: true })),
  openContentRiskRecoveryConfirmation: vi.fn(() => ({ result: Promise.resolve(true), close: vi.fn() })),
  openContentRiskRecoveryUnavailable: vi.fn(() => ({ result: Promise.resolve(false), close: vi.fn() })),
  adapter: {} as Record<string, unknown>,
  controller: undefined as unknown,
}))

vi.mock('../service/composer-resume', () => ({ resumeComposer: mocks.resumeComposer }))
vi.mock('../components/content-risk-recovery-dialog', () => ({
  openContentRiskRecoveryConfirmation: mocks.openContentRiskRecoveryConfirmation,
  openContentRiskRecoveryUnavailable: mocks.openContentRiskRecoveryUnavailable,
}))

vi.mock('dsh-tauri/client', () => {
  const defineLocale = (namespace: string) => ({
    NS: namespace,
    text: (key: string) => key,
    activeLocale: () => 'en',
    isEnglishLocale: () => true,
    useLocale: () => 'en',
    registerLocale: () => () => {},
  })
  const createLifecycleController = () => {
    const disposers: Array<() => void> = []
    let clickHandler: ((event: unknown) => void) | undefined
    let observerCallback: (() => void) | undefined
    let disposed = false
    return {
      add: (disposer: () => void) => {
        disposers.push(disposer)
      },
      listen: (_type: string, handler: (event: unknown) => void) => {
        clickHandler = handler
        return () => {}
      },
      observe: (_node: unknown, callback: () => void) => {
        observerCallback = callback
        return { disconnect: () => {} }
      },
      isDisposed: () => disposed,
      dispose: () => {
        disposed = true
        for (const disposer of [...disposers])
          disposer()
        disposers.length = 0
      },
      triggerObserve: () => observerCallback?.(),
      click: (event: unknown) => clickHandler?.(event),
    }
  }
  return {
    ofetch: vi.fn(),
    defineLocale,
    createLifecycleController,
    defineRegister: (ctxOrSetup: unknown, maybeSetup?: unknown) => {
      const setup = (typeof maybeSetup === 'function' ? maybeSetup : ctxOrSetup) as
        (controller: unknown, ctx: unknown, adapter: unknown) => void
      return function registerEffect(this: unknown) {
        const controller = createLifecycleController()
        mocks.controller = controller
        setup(controller, this, mocks.adapter)
        return () => controller.dispose()
      }
    },
  }
})

class FakeElement {
  closest: (selector: string) => unknown = () => null
}

class FakeButtonElement extends FakeElement {}

interface FakeIcon {
  d: string | null
}

interface FakeIconElement {
  getAttribute: (name: string) => string | null
  setAttribute: (name: string, value: string) => void
}

interface FakeButton {
  disabled: boolean
  querySelector: (selector: string) => FakeIconElement | null
  getAttribute: (name: string) => string | null
  setAttribute: (name: string, value: string) => void
  removeAttribute: (name: string) => void
}

interface FakeCard {
  querySelector: (selector: string) => unknown
  querySelectorAll: (selector: string) => { length: number, item: (index: number) => FakeButton | null }
}

function snapshotSource<T>(initial: T) {
  let state = initial
  const listeners = new Set<() => void>()
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    publish: (next: T) => {
      state = next
      for (const listener of [...listeners])
        listener()
    },
    setSilently: (next: T) => {
      state = next
    },
  }
}

function turnEnd(kind: string): ComposerSessionEventEntry {
  return { type: 'event', event: { type: 'turn/end', data: { reason: { kind } } } }
}

function turnStart(): ComposerSessionEventEntry {
  return { type: 'event', event: { type: 'turn/start' } }
}

function contentRiskEntries(): ComposerSessionEventEntry[] {
  return [
    { type: 'event', event: { type: 'turn/end', seq: 0, data: { turn: 1, reason: { kind: 'completed' } } } },
    { type: 'event', event: { type: 'agent/inbox/spliced', seq: 1, data: { inserted: [{ id: 'request-1' }] } } },
    { type: 'event', event: { type: 'turn/start', seq: 2, data: { turn: 2 } } },
    { type: 'event', event: { type: 'user/message', seq: 3, data: { id: 'request-1' } } },
    {
      type: 'event',
      event: {
        type: 'turn/end',
        seq: 4,
        data: {
          turn: 2,
          reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
        },
      },
    },
  ]
}

function repeatedContentRiskEntries(): ComposerSessionEventEntry[] {
  return [
    ...contentRiskEntries(),
    { type: 'event', event: { type: 'agent/inbox/spliced', seq: 5, data: { inserted: [{ id: 'harmless-retry' }] } } },
    { type: 'event', event: { type: 'turn/start', seq: 6, data: { turn: 3 } } },
    { type: 'event', event: { type: 'user/message', seq: 7, data: { id: 'harmless-retry' } } },
    {
      type: 'event',
      event: {
        type: 'turn/end',
        seq: 8,
        data: {
          turn: 3,
          reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
        },
      },
    },
  ]
}

function createButton(options: { path: string | null, label?: string, disabled?: boolean }) {
  const attributes = new Map<string, string>()
  if (options.label !== undefined)
    attributes.set('aria-label', options.label)
  const icon: FakeIcon = { d: options.path }
  const iconElement: FakeIconElement = {
    getAttribute: (name: string) => name === 'd' ? icon.d : null,
    setAttribute: (name: string, value: string) => {
      if (name === 'd')
        icon.d = value
    },
  }
  let disabled = options.disabled ?? true
  const button: FakeButton = {
    get disabled() {
      return disabled
    },
    set disabled(value: boolean) {
      disabled = value
    },
    querySelector: (selector: string) => selector === 'svg path' ? iconElement : null,
    getAttribute: (name: string) => attributes.get(name) ?? null,
    setAttribute: (name: string, value: string) => void attributes.set(name, value),
    removeAttribute: (name: string) => void attributes.delete(name),
  }
  Object.setPrototypeOf(button, FakeButtonElement.prototype)
  return {
    button,
    icon: () => icon.d,
    label: () => attributes.get('aria-label') ?? null,
  }
}

interface HarnessOptions {
  empty?: boolean
  running?: boolean
  entries?: ComposerSessionEventEntry[]
  path?: string | null
  buttonDisabled?: boolean
  fork?: (options: { sessionId: string, atSeq: number, increaseTitle: boolean }) => Promise<string>
  olderEntries?: ComposerSessionEventEntry[]
  disposeDuringLoadOlder?: boolean
}

function harness(options: HarnessOptions = {}) {
  const sessionId = 's-1'
  const primary = createButton({
    path: options.path === undefined ? ARROW_PATH : options.path,
    label: 'Send message',
    disabled: options.buttonDisabled ?? true,
  })
  const card: FakeCard = {
    querySelector: (selector: string) =>
      selector === '[data-composer-placeholder]' && (options.empty ?? true) ? {} : null,
    querySelectorAll: () => ({ length: 1, item: () => primary.button }),
  }
  const session = snapshotSource<ComposerSessionSnapshot>({
    running: options.running ?? false,
    removed: false,
    subagent: null,
  })
  const events = snapshotSource<{ entries: ComposerSessionEventEntry[], hasMore?: boolean }>({
    entries: options.entries ?? [turnEnd('aborted')],
    hasMore: options.olderEntries !== undefined,
  })
  const list = snapshotSource({ current: sessionId as string | undefined })
  const olderEntries = options.olderEntries
  const loadOlder = olderEntries === undefined
    ? undefined
    : vi.fn(async () => {
        if (options.disposeDuringLoadOlder === true) {
          const controller = mocks.controller as ControllerStub
          controller.dispose()
          return
        }
        events.publish({ entries: olderEntries, hasMore: false })
      })

  mocks.adapter.sessions = {
    list,
    binding: (id: string) => id === sessionId
      ? { session: { ...session, ...(loadOlder === undefined ? {} : { loadOlder }) }, eventSource: events }
      : undefined,
    ...(options.fork === undefined ? {} : { fork: options.fork }),
  }
  mocks.adapter.openSession = vi.fn(() => ({ status: 'opened', value: undefined }))

  Object.assign(globalThis, {
    document: {
      body: {},
      querySelector: (selector: string) => selector === '[data-composer-card]' ? card : null,
    },
    Element: FakeElement,
    HTMLButtonElement: FakeButtonElement,
  })

  const ctx = { locale: { subscribe: () => () => {} } }
  const register = composerResumeFeature as unknown as (this: unknown) => () => void
  const cleanup = register.call(ctx)
  const controller = mocks.controller as ControllerStub

  return {
    sessionId,
    button: primary.button,
    icon: primary.icon,
    label: primary.label,
    title: () => primary.button.getAttribute('title'),
    disabled: () => primary.button.disabled,
    setIcon: (d: string | null) => {
      ;(primary.button.querySelector('svg path') as FakeIconElement).setAttribute('d', d ?? '')
    },
    setRunning: (running: boolean) => session.publish({ running, removed: false, subagent: null }),
    setRunningSilently: (running: boolean) => session.setSilently({ running, removed: false, subagent: null }),
    setEntries: (entries: ComposerSessionEventEntry[]) => events.publish({ entries }),
    triggerObserve: () => controller.triggerObserve(),
    click: (event: unknown) => controller.click(event),
    loadOlder,
    cleanup,
  }
}

beforeEach(() => {
  mocks.resumeComposer.mockClear()
  mocks.resumeComposer.mockResolvedValue({ ok: true })
  mocks.openContentRiskRecoveryConfirmation.mockClear()
  mocks.openContentRiskRecoveryConfirmation.mockReturnValue({ result: Promise.resolve(true), close: vi.fn() })
  mocks.openContentRiskRecoveryUnavailable.mockClear()
  mocks.openContentRiskRecoveryUnavailable.mockReturnValue({ result: Promise.resolve(false), close: vi.fn() })
  delete mocks.adapter.sessions
  delete mocks.adapter.openSession
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('composerResumeFeature', () => {
  it('回合中断后把主按钮改写成可点的 PlayFill', () => {
    const h = harness({ entries: [turnStart(), turnEnd('aborted')] })
    expect(h.icon()).toBe(PLAY_FILL_PATH)
    expect(h.disabled()).toBe(false)
    expect(h.label()).toBe(RESUME_LABEL)
    h.cleanup()
  })

  it('同一按钮被 React 重渲染（新 path + disabled）后再次改写', () => {
    const h = harness({ entries: [turnStart(), turnEnd('aborted')] })
    expect(h.icon()).toBe(PLAY_FILL_PATH)

    h.setIcon(ARROW_PATH)
    h.button.disabled = true
    h.triggerObserve()

    expect(h.icon()).toBe(PLAY_FILL_PATH)
    expect(h.disabled()).toBe(false)
    h.cleanup()
  })

  it('二次中断：running 尚未发布给订阅者时也能判定可继续', () => {
    const h = harness({ running: true, entries: [turnStart()] })
    expect(h.icon()).toBe(ARROW_PATH)

    h.setEntries([turnStart(), turnEnd('aborted')])
    expect(h.icon()).toBe(ARROW_PATH)

    h.setRunningSilently(false)
    h.button.disabled = true
    h.triggerObserve()

    expect(h.icon()).toBe(PLAY_FILL_PATH)
    expect(h.disabled()).toBe(false)
    h.cleanup()
  })

  it('任务进行中不改写（stop 方块阶段不被接管）', () => {
    const h = harness({ running: true, path: null, entries: [turnStart()] })
    expect(h.icon()).toBe(null)
    h.cleanup()
  })

  it('任务正常结束时不改写', () => {
    const h = harness({ entries: [turnStart(), turnEnd('completed')] })
    expect(h.icon()).toBe(ARROW_PATH)
    expect(h.disabled()).toBe(true)
    h.cleanup()
  })

  it('草稿非空时还原内核图标与禁用态', () => {
    const h = harness({ empty: false, entries: [turnStart(), turnEnd('aborted')] })
    expect(h.icon()).toBe(ARROW_PATH)
    expect(h.disabled()).toBe(true)
    h.cleanup()
  })

  it('点击只请求继续，不发送草稿文本', () => {
    const h = harness({ entries: [turnStart(), turnEnd('aborted')] })
    const preventDefault = vi.fn()
    const stopPropagation = vi.fn()
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault, stopPropagation })

    expect(preventDefault).toHaveBeenCalledTimes(1)
    expect(stopPropagation).toHaveBeenCalledTimes(1)
    expect(mocks.resumeComposer).toHaveBeenCalledWith({ sessionId: 's-1' })
    h.cleanup()
  })

  it('内容审核错误只通过安全边界分叉继续', async () => {
    const fork = vi.fn(async () => 'child')
    const h = harness({ entries: contentRiskEntries(), fork })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

    await vi.waitFor(() => expect(mocks.resumeComposer).toHaveBeenCalledTimes(1))
    expect(h.label()).toBe(CONTENT_RISK_RECOVERY_LABEL)
    expect(h.title()).toBe(CONTENT_RISK_RECOVERY_LABEL)
    expect(mocks.openContentRiskRecoveryConfirmation).toHaveBeenCalledWith({
      safeSeq: 0,
      excludedEventCount: 4,
    })
    expect(mocks.resumeComposer).toHaveBeenCalledWith({
      sessionId: 's-1',
      recovery: expect.objectContaining({ atSeq: 0 }),
    })
    h.cleanup()
  })

  it('内核没有分叉能力时拒绝在原会话重放内容审核错误', async () => {
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness({ entries: contentRiskEntries() })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

    await vi.waitFor(() => expect(warning).toHaveBeenCalledWith(expect.stringContaining('拒绝在原会话重放')))
    expect(mocks.openContentRiskRecoveryUnavailable).toHaveBeenCalledTimes(1)
    expect(mocks.resumeComposer).not.toHaveBeenCalled()
    h.cleanup()
    warning.mockRestore()
  })

  it('当前窗口缺少安全边界时先加载更早事件再分叉', async () => {
    const fork = vi.fn(async () => 'child')
    const fullEntries = repeatedContentRiskEntries()
    const h = harness({ entries: fullEntries.slice(4), olderEntries: fullEntries, fork })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

    await vi.waitFor(() => expect(mocks.resumeComposer).toHaveBeenCalledTimes(1))
    expect(h.loadOlder).toHaveBeenCalledTimes(1)
    expect(mocks.resumeComposer).toHaveBeenCalledWith({
      sessionId: 's-1',
      recovery: expect.objectContaining({ atSeq: 0 }),
    })
    h.cleanup()
  })

  it('加载更早事件期间插件被卸载时停止恢复', async () => {
    const fork = vi.fn(async () => 'child')
    const fullEntries = repeatedContentRiskEntries()
    const h = harness({
      entries: fullEntries.slice(4),
      olderEntries: fullEntries,
      fork,
      disposeDuringLoadOlder: true,
    })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

    await vi.waitFor(() => expect(h.loadOlder).toHaveBeenCalledTimes(1))
    expect(mocks.resumeComposer).not.toHaveBeenCalled()
    expect(fork).not.toHaveBeenCalled()
    expect(mocks.openContentRiskRecoveryUnavailable).not.toHaveBeenCalled()
    h.cleanup()
  })

  it('用户取消安全分支确认时不创建分支', async () => {
    mocks.openContentRiskRecoveryConfirmation.mockReturnValueOnce({
      result: Promise.resolve(false),
      close: vi.fn(),
    })
    const fork = vi.fn(async () => 'child')
    const h = harness({ entries: contentRiskEntries(), fork })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })

    await vi.waitFor(() => expect(mocks.openContentRiskRecoveryConfirmation).toHaveBeenCalledTimes(1))
    expect(mocks.resumeComposer).not.toHaveBeenCalled()
    expect(fork).not.toHaveBeenCalled()
    h.cleanup()
  })

  it('确认对话框打开期间插件被卸载时停止恢复', async () => {
    let resolveConfirmation: (accepted: boolean) => void = () => {}
    const confirmation = new Promise<boolean>((resolve) => {
      resolveConfirmation = resolve
    })
    mocks.openContentRiskRecoveryConfirmation.mockReturnValueOnce({
      result: confirmation,
      close: vi.fn(),
    })
    const fork = vi.fn(async () => 'child')
    const h = harness({ entries: contentRiskEntries(), fork })
    const target = new FakeElement()
    target.closest = () => h.button

    h.click({ target, preventDefault: vi.fn(), stopPropagation: vi.fn() })
    await vi.waitFor(() => expect(mocks.openContentRiskRecoveryConfirmation).toHaveBeenCalledTimes(1))
    h.cleanup()
    resolveConfirmation(true)
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(mocks.resumeComposer).not.toHaveBeenCalled()
    expect(fork).not.toHaveBeenCalled()
  })
})
