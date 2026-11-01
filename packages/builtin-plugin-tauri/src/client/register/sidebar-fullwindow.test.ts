// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { createLifecycleController } from '../controller'
import { navigationFeature } from './navigation'
import { FULL_SIDEBAR_ATTRIBUTE, presentFullSidebar } from './sidebar-fullwindow'

const { emit } = vi.hoisted(() => ({ emit: vi.fn().mockResolvedValue(undefined) }))
vi.mock('../service/tauri', () => ({ emit, listen: vi.fn().mockResolvedValue(() => {}), invoke: vi.fn() }))
let dispose: (() => void) | undefined

afterEach(() => {
  dispose?.()
  dispose = undefined
  document.body.replaceChildren()
  document.body.style.removeProperty('visibility')
  document.documentElement.removeAttribute(FULL_SIDEBAR_ATTRIBUTE)
  delete (window as Window & { __DSH_NAVIGATION__?: unknown }).__DSH_NAVIGATION__
  vi.useRealTimers()
  vi.clearAllMocks()
})

it('reveals a full sidebar when its navigation register is disposed before completion and reentered', () => {
  vi.useFakeTimers()
  const host = window as Window & { __DSH_NAVIGATION__?: unknown }
  host.__DSH_NAVIGATION__ = { presentation: 'full-sidebar' }
  dispose = navigationFeature.call({})
  expect(host.__DSH_NAVIGATION__).toBeUndefined()
  expect(document.body.style.visibility).toBe('hidden')
  dispose()
  dispose = navigationFeature.call({})
  vi.advanceTimersByTime(10_000)
  expect(document.body.style.visibility).toBe('')
  expect(document.documentElement.getAttribute(FULL_SIDEBAR_ATTRIBUTE)).toBe('ready')
  expect(emit).toHaveBeenCalledExactlyOnceWith('dsh://full-sidebar:ready')
  expect(vi.getTimerCount()).toBe(0)
})

it('expands a collapsed sidebar before using its already mounted fullscreen control', () => {
  vi.useFakeTimers()
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
