import { afterEach, describe, expect, it, vi } from 'vitest'
import { detectMobileDevice, isMobileDevice } from './settings.utils'

const MOBILE_QUERIES = ['(hover: none)', '(any-pointer: coarse)', '(any-hover: none)']

/** 只记查询串、按给定的答案表作答：被测函数不碰真实浏览器。 */
function matcher(answers: Record<string, boolean>) {
  const queries: string[] = []
  const matches = (query: string): boolean => {
    queries.push(query)
    return answers[query] ?? false
  }
  return {
    queries,
    matches,
    matchMedia: (query: string) => ({ matches: matches(query) }),
  }
}

function stubWindow(matchMedia?: (query: string) => { matches: boolean }): void {
  vi.stubGlobal('window', matchMedia === undefined ? undefined : { matchMedia })
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('isMobileDevice', () => {
  it('treats a touch device with a coarse pointer as mobile', () => {
    const probe = matcher({
      '(hover: none)': true,
      '(any-pointer: coarse)': true,
      '(any-hover: none)': true,
    })

    expect(isMobileDevice(probe.matches)).toBe(true)
    expect(probe.queries).toEqual(MOBILE_QUERIES)
  })

  /** 触屏笔记本两个都成立、纯触屏平板与鼠标桌面各只成立一个，都不能进手机端分支。 */
  it('keeps every partially matching device on the desktop branch', () => {
    const mouseDesktop = matcher({ '(hover: none)': false, '(any-pointer: coarse)': false })
    const touchscreenLaptop = matcher({ '(hover: none)': false, '(any-pointer: coarse)': true })
    const hoverTouch = matcher({ '(hover: none)': true, '(any-pointer: coarse)': false })

    expect(isMobileDevice(mouseDesktop.matches)).toBe(false)
    expect(isMobileDevice(touchscreenLaptop.matches)).toBe(false)
    expect(isMobileDevice(hoverTouch.matches)).toBe(false)
  })

  /** 触屏为主、另接鼠标的设备（`any-hover: hover`）有鼠标可用，仍按桌面端处理。 */
  it('keeps a touchscreen with a secondary mouse on the desktop branch', () => {
    const tabletWithMouse = matcher({
      '(hover: none)': true,
      '(any-pointer: coarse)': true,
      '(any-hover: hover)': true,
    })

    expect(isMobileDevice(tabletWithMouse.matches)).toBe(false)
    expect(tabletWithMouse.queries).toEqual(MOBILE_QUERIES)
  })

  /** 判据只看设备能力：窄窗口但带鼠标仍是桌面端，横竖屏切换不会翻案。 */
  it('ignores viewport width', () => {
    const narrowDesktop = matcher({
      '(max-width: 480px)': true,
      '(hover: none)': false,
      '(any-pointer: coarse)': false,
    })

    expect(isMobileDevice(narrowDesktop.matches)).toBe(false)
    expect(narrowDesktop.queries).not.toContain('(max-width: 480px)')
  })
})

describe('detectMobileDevice', () => {
  it('probes the device capabilities through the kernel matchMedia', () => {
    const probe = matcher({
      '(hover: none)': true,
      '(any-pointer: coarse)': true,
      '(any-hover: none)': true,
    })
    stubWindow(probe.matchMedia)

    expect(detectMobileDevice()).toBe(true)
    expect(probe.queries).toEqual(MOBILE_QUERIES)
  })

  /** 老内核没有 matchMedia（node 单测同样如此）：按桌面端处理，保住自有侧栏。 */
  it('falls back to the desktop branch without a kernel matchMedia', () => {
    stubWindow()

    expect(detectMobileDevice()).toBe(false)
  })
})
