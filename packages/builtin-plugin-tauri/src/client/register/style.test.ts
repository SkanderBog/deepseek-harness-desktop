// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { registerStyle } from './style'

const { emit } = vi.hoisted(() => ({ emit: vi.fn().mockResolvedValue(undefined) }))
vi.mock('../service/tauri', () => ({ emit }))
let dispose: (() => void) | undefined

afterEach(() => {
  dispose?.()
  dispose = undefined
  document.body.replaceChildren()
  document.documentElement.style.removeProperty('color-scheme')
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

it.each([
  ['mask', 'hidden', ''],
  ['mask', 'style', 'display: none'],
  ['mask', 'style', 'visibility: hidden'],
  ['ancestor', 'hidden', ''],
  ['ancestor', 'style', 'display: none'],
  ['ancestor', 'style', 'visibility: hidden'],
])('ignores a %s hidden with %s="%s" and reports visibility changes', async (target, attribute, value) => {
  vi.useFakeTimers()
  document.body.innerHTML = '<section><div style="position: fixed"><div class="_mask_fixed" style="background: rgba(0, 0, 0, 0.5); inset: 0px"></div></div></section><div class="_mask_visible" style="background: rgba(0, 0, 0, 0.3); inset: 0px"></div>'
  const mask = document.querySelector<HTMLElement>('._mask_fixed')!
  const hidden = target === 'mask' ? mask : mask.parentElement!.parentElement!
  const visible = document.querySelector<HTMLElement>('._mask_visible')!
  const original = hidden.getAttribute(attribute)
  hidden.setAttribute(attribute, attribute === 'style' ? [original, value].filter(Boolean).join('; ') : value)
  const computed = globalThis.getComputedStyle
  vi.spyOn(globalThis, 'getComputedStyle').mockImplementation(element => computed(element))
  dispose = registerStyle.call({})
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(1)
  expect(emit.mock.lastCall).toEqual(['dsh://style', expect.objectContaining({
    marked: expect.objectContaining({ background: 'rgba(0, 0, 0, 0.3)' }),
  })])

  visible.hidden = true
  await Promise.resolve()
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(2)
  expect(emit.mock.lastCall).toEqual(['dsh://style', expect.objectContaining({ marked: null })])

  visible.hidden = false
  await Promise.resolve()
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(3)
  expect(emit.mock.lastCall).toEqual(['dsh://style', expect.objectContaining({
    marked: expect.objectContaining({ background: 'rgba(0, 0, 0, 0.3)' }),
  })])

  if (original === null)
    hidden.removeAttribute(attribute)
  else
    hidden.setAttribute(attribute, original)
  await Promise.resolve()
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(4)
  expect(emit.mock.lastCall).toEqual(['dsh://style', expect.objectContaining({
    marked: expect.objectContaining({ background: 'rgba(0, 0, 0, 0.5)' }),
  })])
})

it('reports theme and painted layers, coalesces relevant changes, and cancels reports on disposal', async () => {
  vi.useFakeTimers()
  document.documentElement.style.colorScheme = 'light'
  document.body.innerHTML = '<div class="app_frame" style="background: rgb(10, 20, 30)"></div><section data-slot="sidebar"><div style="background: rgb(40, 50, 60)"></div></section><div style="position: fixed"><div class="_mask_dialog" style="inset: 0px"></div></div>'
  const after = document.createElement('div').style
  after.background = 'rgba(0, 0, 0, 0.5)'
  after.animationDuration = '0.2s, 1s'
  after.animationTimingFunction = 'linear'
  const computed = globalThis.getComputedStyle
  vi.spyOn(globalThis, 'getComputedStyle').mockImplementation((element, pseudo) => {
    if (pseudo === '::after')
      return after
    const style = computed(element)
    if (element.matches('._mask_dialog'))
      return Object.assign(style, { backdropFilter: 'blur(8px)' })
    return style
  })
  const frame = document.querySelector<HTMLDivElement>('.app_frame')!
  const sidebar = document.querySelector<HTMLDivElement>('[data-slot="sidebar"] > div')!
  dispose = registerStyle.call({})
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledExactlyOnceWith('dsh://style', {
    colorScheme: 'light',
    frame: { background: 'rgb(10, 20, 30)' },
    sidebar: { background: 'rgb(40, 50, 60)' },
    marked: {
      inset: '0px',
      background: 'rgba(0, 0, 0, 0.5)',
      backdropFilter: 'blur(8px)',
      bottom: '-1px',
      transitionProperty: 'background, backdrop-filter',
      transitionDuration: '0.2s',
      transitionTimingFunction: 'linear',
    },
  })
  window.dispatchEvent(new Event('resize'))
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(1)
  frame.style.background = 'red'
  sidebar.style.background = 'blue'
  document.documentElement.style.colorScheme = 'dark'
  await Promise.resolve()
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(2)
  expect(emit.mock.lastCall).toEqual(['dsh://style', expect.objectContaining({
    colorScheme: 'dark',
    frame: { background: 'rgb(255, 0, 0)' },
    sidebar: { background: 'rgb(0, 0, 255)' },
  })])
  document.body.replaceChildren()
  await Promise.resolve()
  vi.advanceTimersByTime(20)
  expect(emit.mock.lastCall).toEqual(['dsh://style', { colorScheme: 'dark', frame: null, sidebar: null, marked: null }])
  const calls = emit.mock.calls.length
  window.dispatchEvent(new Event('resize'))
  dispose()
  vi.advanceTimersByTime(20)
  window.dispatchEvent(new Event('resize'))
  vi.advanceTimersByTime(20)
  expect(emit).toHaveBeenCalledTimes(calls)
})
