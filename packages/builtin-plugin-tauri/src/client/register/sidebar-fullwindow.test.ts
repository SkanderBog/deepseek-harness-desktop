// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { createLifecycleController } from '../controller'
import { navigationFeature } from './navigation'
import { FULL_SIDEBAR_ATTRIBUTE, presentFullSidebar } from './sidebar-fullwindow'

const { emit } = vi.hoisted(() => ({ emit: vi.fn().mockResolvedValue(undefined) }))
vi.mock('../service/tauri', () => ({ emit, listen: vi.fn().mockResolvedValue(() => {}), invoke: vi.fn().mockResolvedValue(undefined) }))
let dispose: (() => void) | undefined

afterEach(() => {
  dispose?.()
  dispose = undefined
  document.body.replaceChildren()
  document.body.style.removeProperty('visibility')
  document.documentElement.removeAttribute(FULL_SIDEBAR_ATTRIBUTE)
  document.documentElement.removeAttribute('data-dsh-tauri-layout-isolated')
  window.sessionStorage.clear()
  delete (window as Window & { __DSH_NAVIGATION__?: unknown }).__DSH_NAVIGATION__
  vi.useRealTimers()
  vi.clearAllMocks()
})

it('cancels before the first poll without reporting readiness and keeps pending navigation', () => {
  vi.useFakeTimers()
  const host = window as Window & { __DSH_NAVIGATION__?: unknown }
  const navigation = { sessionId: 'pending-sidebar', presentation: 'full-sidebar' }
  host.__DSH_NAVIGATION__ = navigation
  dispose = navigationFeature.call({})
  expect(document.body.style.visibility).toBe('hidden')
  dispose()
  vi.advanceTimersByTime(10_000)
  expect(document.body.style.visibility).toBe('')
  expect(document.documentElement.getAttribute(FULL_SIDEBAR_ATTRIBUTE)).not.toBe('ready')
  expect(emit).not.toHaveBeenCalled()
  expect(host.__DSH_NAVIGATION__).toEqual(navigation)
  expect(vi.getTimerCount()).toBe(0)
})

it('resumes pending navigation after reentry and reports ready only after expansion', async () => {
  vi.useFakeTimers()
  const host = window as Window & { __DSH_NAVIGATION__?: unknown }
  const sessionId = 'pending-sidebar'
  host.__DSH_NAVIGATION__ = { sessionId, presentation: 'full-sidebar' }
  dispose = navigationFeature.call({})
  dispose()
  emit.mockClear()
  const openSession = vi.fn()
  dispose = navigationFeature.call({
    sessions: { list: { getSnapshot: () => ({ ids: [sessionId] }), subscribe: () => () => {} } },
    uiWorkspace: { openSession },
  })
  expect(document.body.style.visibility).toBe('hidden')
  vi.advanceTimersByTime(500)
  expect(emit).not.toHaveBeenCalled()
  const panel = mountCollapsedSidebar()
  await vi.advanceTimersByTimeAsync(750)
  expect(openSession).toHaveBeenCalledExactlyOnceWith(sessionId)
  expect(panel.hasAttribute('data-sidebar-right-open')).toBe(true)
  expect(panel.getAttribute('data-sidebar-right-panel')).toBe('fullscreen')
  expect(document.body.style.visibility).toBe('')
  expect(emit).toHaveBeenCalledExactlyOnceWith('dsh://full-sidebar:ready')
  dispose()
  expect(host.__DSH_NAVIGATION__).toBeUndefined()
  expect(vi.getTimerCount()).toBe(0)
})

function mountCollapsedSidebar(): HTMLElement {
  document.body.innerHTML = '<button data-sidebar-right-expand></button><div data-sidebar-right-panel="push" aria-hidden="true"><button data-sidebar-right-mode="fullscreen"></button></div>'
  const expand = document.querySelector<HTMLButtonElement>('[data-sidebar-right-expand]')!
  const panel = document.querySelector<HTMLElement>('[data-sidebar-right-panel]')!
  const mode = document.querySelector<HTMLButtonElement>('[data-sidebar-right-mode]')!
  expand.addEventListener('click', () => {
    expand.remove()
    panel.setAttribute('data-sidebar-right-open', '')
    panel.setAttribute('aria-hidden', 'false')
  })
  mode.addEventListener('click', () => {
    if (panel.hasAttribute('data-sidebar-right-open')) {
      panel.setAttribute('data-sidebar-right-panel', 'fullscreen')
      mode.setAttribute('data-sidebar-right-mode', 'push')
    }
  })
  return panel
}

it('expands a collapsed sidebar before using its already mounted fullscreen control', () => {
  vi.useFakeTimers()
  const panel = mountCollapsedSidebar()
  const controller = createLifecycleController()
  dispose = () => controller.dispose()
  presentFullSidebar(controller)
  vi.advanceTimersByTime(750)
  expect(panel.hasAttribute('data-sidebar-right-open')).toBe(true)
  expect(panel.getAttribute('data-sidebar-right-panel')).toBe('fullscreen')
  expect(document.body.style.visibility).toBe('')
  expect(emit).toHaveBeenCalledExactlyOnceWith('dsh://full-sidebar:ready')
  expect(vi.getTimerCount()).toBe(0)
})

it('restores visibility on presentation timeout without reporting readiness', () => {
  vi.useFakeTimers()
  const controller = createLifecycleController()
  dispose = () => controller.dispose()
  presentFullSidebar(controller)
  vi.advanceTimersByTime(8250)
  expect(document.body.style.visibility).toBe('')
  expect(document.documentElement.getAttribute(FULL_SIDEBAR_ATTRIBUTE)).not.toBe('ready')
  expect(emit).not.toHaveBeenCalled()
  expect(vi.getTimerCount()).toBe(0)
  dispose()
  expect(emit).not.toHaveBeenCalled()
})

it('reveals a completed full sidebar once and releases the presentation poll', () => {
  vi.useFakeTimers()
  document.body.innerHTML = '<button data-sidebar-right-mode="inline"></button>'
  const controller = createLifecycleController()
  dispose = () => controller.dispose()
  presentFullSidebar(controller)
  vi.advanceTimersByTime(250)
  expect(document.body.style.visibility).toBe('')
  expect(document.documentElement.getAttribute(FULL_SIDEBAR_ATTRIBUTE)).toBe('ready')
  dispose()
  expect(emit).toHaveBeenCalledExactlyOnceWith('dsh://full-sidebar:ready')
  expect(vi.getTimerCount()).toBe(0)
})
