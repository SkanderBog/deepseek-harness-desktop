import type { ComposerSessionEventEntry } from './composer-resume.types'
import { describe, expect, it } from 'vitest'
import { contentRiskRecoveryBoundary, isComposerEmpty, lastTurnEndReason, paintResumeIcon, readIconPath, restoreDisabled, restorePrimaryIcon, shouldOfferResume } from './composer-resume.utils'

const ARROW_PATH = 'M8.3125 0.980183C8.66767 1.0531'
const PLAY_FILL_PATH = 'M14.642 6.285c1.294.777 1.294 2.653 0 3.43l-9.113 5.468c-1.333.8-3.028-.16-3.029-1.715V2.532C2.5.978 4.196.018 5.53.818z'

interface ButtonStub {
  button: HTMLButtonElement
  attributes: Map<string, string>
  iconPath: () => string | null
  pathWrites: () => number
  disabledWrites: () => number
}

function stubButton(options: { path?: string, ariaLabel?: string, noPath?: boolean, disabled?: boolean } = {}): ButtonStub {
  const attributes = new Map<string, string>()
  if (options.path !== undefined)
    attributes.set('d', options.path)
  if (options.ariaLabel !== undefined)
    attributes.set('aria-label', options.ariaLabel)

  let pathWrites = 0
  let disabledWrites = 0
  let disabled = options.disabled ?? true

  const pathElement = {
    getAttribute: (name: string) => attributes.get(name) ?? null,
    setAttribute: (name: string, value: string) => {
      pathWrites += 1
      attributes.set(name, value)
    },
  }
  const button = {
    get disabled() {
      return disabled
    },
    set disabled(value: boolean) {
      disabledWrites += 1
      disabled = value
    },
    querySelector: (selector: string) => selector === 'svg path' && options.noPath !== true ? pathElement : null,
    getAttribute: (name: string) => attributes.get(name) ?? null,
    setAttribute: (name: string, value: string) => void attributes.set(name, value),
    removeAttribute: (name: string) => void attributes.delete(name),
  }
  return {
    button: button as unknown as HTMLButtonElement,
    attributes,
    iconPath: () => attributes.get('d') ?? null,
    pathWrites: () => pathWrites,
    disabledWrites: () => disabledWrites,
  }
}

function stubCard(placeholder: boolean): Element {
  return { querySelector: () => (placeholder ? {} : null) } as unknown as Element
}

describe('isComposerEmpty', () => {
  it('reads the kernel placeholder as the empty-draft signal', () => {
    expect(isComposerEmpty(stubCard(true))).toBe(true)
    expect(isComposerEmpty(stubCard(false))).toBe(false)
  })
})

describe('lastTurnEndReason', () => {
  function entry(type: string, kind?: string): ComposerSessionEventEntry {
    return { type: 'event', event: { type, data: kind === undefined ? {} : { reason: { kind } } } }
  }

  it('returns the newest turn/end reason kind', () => {
    expect(lastTurnEndReason([
      entry('turn/start'),
      entry('turn/end', 'completed'),
      entry('turn/start'),
      entry('turn/end', 'aborted'),
    ])).toEqual({ kind: 'aborted' })
  })

  it('reads the interrupted kind synthesized by crash-tail recovery', () => {
    expect(lastTurnEndReason([entry('turn/end', 'interrupted')])).toEqual({ kind: 'interrupted' })
  })

  it('ignores an open turn, transient entries and missing windows', () => {
    expect(lastTurnEndReason([entry('turn/end', 'error'), entry('turn/start')])).toEqual({ kind: 'error' })
    expect(lastTurnEndReason([{ type: 'transient', event: { type: 'assistant/live-chunk' } }])).toBeUndefined()
    expect(lastTurnEndReason([])).toBeUndefined()
    expect(lastTurnEndReason(undefined)).toBeUndefined()
  })

  it('returns the raw kind without narrowing it', () => {
    expect(lastTurnEndReason([entry('turn/end', 'max-tokens')])).toEqual({ kind: 'max-tokens' })
  })
})

describe('contentRiskRecoveryBoundary', () => {
  const event = (seq: number, type: string, data: NonNullable<ComposerSessionEventEntry['event']>['data'] = {}): ComposerSessionEventEntry => ({
    type: 'event',
    event: { type, seq, data },
  })

  it('cuts immediately before the inbox insertion claimed by the rejected turn', () => {
    const entries = [
      event(0, 'turn/end', { turn: 1, reason: { kind: 'completed' } }),
      event(1, 'agent/inbox/spliced', { inserted: [{ id: 'request-1' }] }),
      event(2, 'turn/start', { turn: 2 }),
      event(3, 'agent/inbox/spliced', { inserted: [] }),
      event(4, 'user/message', { id: 'request-1', turn: 2 }),
      event(5, 'tool/result', { turn: 2 }),
      event(6, 'turn/end', {
        turn: 2,
        reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
      }),
    ]

    expect(contentRiskRecoveryBoundary(entries)).toBe(0)
  })

  it('uses the earliest claimed input when a turn admits more than one inbox message', () => {
    const entries = [
      event(4, 'turn/end', { turn: 1, reason: { kind: 'completed' } }),
      event(5, 'agent/inbox/spliced', { inserted: [{ id: 'context-1' }] }),
      event(6, 'agent/inbox/spliced', { inserted: [{ id: 'request-1' }] }),
      event(7, 'turn/start', { turn: 2 }),
      event(8, 'user/message', { id: 'context-1', turn: 2 }),
      event(9, 'user/message', { id: 'request-1', turn: 2 }),
      event(10, 'turn/end', {
        turn: 2,
        reason: { kind: 'error', error: { code: 'CONTENT_REJECTED', message: 'policy rejection' } },
      }),
    ]

    expect(contentRiskRecoveryBoundary(entries)).toBe(4)
  })

  it('rewinds to the first failed turn after the last successful model turn', () => {
    const entries = [
      event(0, 'turn/end', { turn: 1, reason: { kind: 'completed' } }),
      event(1, 'agent/inbox/spliced', { inserted: [{ id: 'poisoned-request' }] }),
      event(2, 'turn/start', { turn: 2 }),
      event(3, 'user/message', { id: 'poisoned-request' }),
      event(4, 'tool/result', { turn: 2 }),
      event(5, 'turn/end', {
        turn: 2,
        reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
      }),
      event(6, 'agent/inbox/spliced', { inserted: [{ id: 'harmless-retry' }] }),
      event(7, 'turn/start', { turn: 3 }),
      event(8, 'user/message', { id: 'harmless-retry' }),
      event(9, 'turn/end', {
        turn: 3,
        reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
      }),
    ]

    expect(contentRiskRecoveryBoundary(entries)).toBe(0)
  })

  it('does not trust a later retry boundary while older history remains unloaded', () => {
    const entries = [
      event(5, 'turn/end', {
        turn: 2,
        reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
      }),
      event(6, 'agent/inbox/spliced', { inserted: [{ id: 'harmless-retry' }] }),
      event(7, 'turn/start', { turn: 3 }),
      event(8, 'user/message', { id: 'harmless-retry' }),
      event(9, 'turn/end', {
        turn: 3,
        reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'Content Exists Risk', status: 400 } },
      }),
    ]

    expect(contentRiskRecoveryBoundary(entries, { historyComplete: false })).toBeUndefined()
  })

  it('does not guess a cut for unrelated errors or an incomplete event window', () => {
    expect(contentRiskRecoveryBoundary([
      event(0, 'agent/inbox/spliced', { inserted: [{ id: 'request-1' }] }),
      event(1, 'turn/start', { turn: 1 }),
      event(2, 'user/message', { id: 'request-1', turn: 1 }),
      event(3, 'turn/end', { turn: 1, reason: { kind: 'error', error: { code: 'INVALID_REQUEST', message: 'invalid model' } } }),
    ])).toBeUndefined()
    expect(contentRiskRecoveryBoundary([
      event(2, 'turn/start', { turn: 2 }),
      event(3, 'user/message', { id: 'request-1', turn: 2 }),
      event(4, 'turn/end', { turn: 2, reason: { kind: 'error', error: { code: 'CONTENT_REJECTED' } } }),
    ])).toBeUndefined()
  })
})

describe('paintResumeIcon / restorePrimaryIcon', () => {
  it('paints the play icon, enables the button and restores the original state', () => {
    const stub = stubButton({ ariaLabel: 'Send message', path: ARROW_PATH })
    expect(readIconPath(stub.button)).toBe(ARROW_PATH)

    paintResumeIcon(stub.button, 'Resume task')
    expect(stub.iconPath()).toBe(PLAY_FILL_PATH)
    expect(stub.button.disabled).toBe(false)
    expect(stub.attributes.get('aria-label')).toBe('Resume task')

    restorePrimaryIcon(stub.button, { path: ARROW_PATH, ariaLabel: 'Send message' }, { label: 'Resume task', disabled: true })
    expect(stub.iconPath()).toBe(ARROW_PATH)
    expect(stub.attributes.get('aria-label')).toBe('Send message')
    expect(stub.button.disabled).toBe(true)
  })

  it('leaves a kernel re-rendered icon and label alone', () => {
    const stub = stubButton({ ariaLabel: 'Steer the running turn', path: 'M8 0.75 0 0 1', disabled: false })
    restorePrimaryIcon(stub.button, { path: ARROW_PATH, ariaLabel: 'Send message' }, { label: 'Resume task', disabled: false })
    expect(stub.iconPath()).toBe('M8 0.75 0 0 1')
    expect(stub.attributes.get('aria-label')).toBe('Steer the running turn')
    expect(stub.button.disabled).toBe(false)
  })

  it('keeps the button enabled when the draft is no longer empty', () => {
    const stub = stubButton({ ariaLabel: 'Send message', path: ARROW_PATH, disabled: false })
    paintResumeIcon(stub.button, 'Resume task')
    restorePrimaryIcon(stub.button, { path: ARROW_PATH, ariaLabel: 'Send message' }, { label: 'Resume task', disabled: false })
    expect(stub.button.disabled).toBe(false)
  })

  it('drops an aria-label the kernel never set', () => {
    const stub = stubButton({ path: ARROW_PATH })
    paintResumeIcon(stub.button, 'Resume task')
    restorePrimaryIcon(stub.button, { path: ARROW_PATH, ariaLabel: null }, { label: 'Resume task', disabled: false })
    expect(stub.attributes.has('aria-label')).toBe(false)
  })

  it('re-applies the play icon without writing the same value twice', () => {
    const stub = stubButton({ ariaLabel: 'Send message', path: ARROW_PATH, disabled: false })
    paintResumeIcon(stub.button, 'Resume task')
    const firstWrites = stub.pathWrites()
    const firstDisabled = stub.disabledWrites()

    paintResumeIcon(stub.button, 'Resume task')
    paintResumeIcon(stub.button, 'Resume task')

    expect(firstWrites).toBe(1)
    expect(stub.pathWrites()).toBe(firstWrites)
    expect(stub.disabledWrites()).toBe(firstDisabled)
    expect(stub.iconPath()).toBe(PLAY_FILL_PATH)
  })

  it('tolerates a button whose icon node is not mounted yet', () => {
    const stub = stubButton({ ariaLabel: 'Send message', noPath: true, disabled: true })
    paintResumeIcon(stub.button, 'Resume task')
    expect(stub.iconPath()).toBe(null)
    expect(stub.button.disabled).toBe(false)
    expect(stub.attributes.get('aria-label')).toBe('Resume task')
  })

  it('settles the resume offer from the live snapshot, never from a cached flag', () => {
    const entries: ComposerSessionEventEntry[] = [{ type: 'event', event: { type: 'turn/end', data: { reason: { kind: 'aborted' } } } }]
    expect(shouldOfferResume({ session: { running: false, removed: false }, entries })).toBe(true)
    expect(shouldOfferResume({ session: { running: true, removed: false }, entries })).toBe(false)
    expect(shouldOfferResume({ session: { running: false, removed: true }, entries })).toBe(false)
    expect(shouldOfferResume({ session: undefined, entries })).toBe(false)
    expect(shouldOfferResume({ session: { running: false }, entries: [] })).toBe(false)
  })

  it('leaves the kernel stop button enabled while the turn runs', () => {
    expect(restoreDisabled(true, false, false)).toBe(true)
    expect(restoreDisabled(true, true, false)).toBe(false)
    expect(restoreDisabled(true, true, true)).toBe(true)
    expect(restoreDisabled(false, false, false)).toBe(false)
  })
})
