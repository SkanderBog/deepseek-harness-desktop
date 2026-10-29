// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import config from '../src-tauri/tauri.conf.json'
import App from '../src/App'

const native = vi.hoisted(() => ({
  enabled: true,
  resize: () => {},
  isMaximized: vi.fn(),
  isFullscreen: vi.fn(),
  onResized: vi.fn(),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  unlisten: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => native.enabled }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => native }))

beforeEach(() => {
  native.enabled = true
  native.isMaximized.mockResolvedValue(false)
  native.isFullscreen.mockResolvedValue(false)
  native.onResized.mockImplementation(async (listener) => {
    native.resize = listener
    return native.unlisten
  })
  native.minimize.mockResolvedValue(undefined)
  native.toggleMaximize.mockResolvedValue(undefined)
  native.close.mockResolvedValue(undefined)
})

afterEach(() => {
  cleanup()
  vi.resetAllMocks()
})

it('基础布局只保留顶部窗口控件和空内容区，Electron 随包安装', async () => {
  const { unmount } = render(<App />)
  expect(screen.getAllByRole('button')).toHaveLength(3)
  expect(screen.getByTestId('dsh-shell-content').childElementCount).toBe(0)
  expect(config.bundle.resources).toEqual({ 'target/electron': 'runtime' })
  expect(config.app.windows[0].decorations).toBe(false)

  fireEvent.click(screen.getByRole('button', { name: '最小化' }))
  fireEvent.click(screen.getByRole('button', { name: '最大化' }))
  fireEvent.click(screen.getByRole('button', { name: '关闭' }))
  expect(native.minimize).toHaveBeenCalledTimes(1)
  expect(native.toggleMaximize).toHaveBeenCalledTimes(1)
  expect(native.close).toHaveBeenCalledTimes(1)

  await act(async () => {
    native.isMaximized.mockResolvedValue(true)
    native.resize()
  })
  expect(screen.getByRole('button', { name: '还原' }).title).toBe('还原')
  unmount()
  await act(async () => {})
  expect(native.unlisten).toHaveBeenCalledTimes(1)
})

it('浏览器预览不调用原生窗口接口', () => {
  native.enabled = false
  render(<App />)
  fireEvent.click(screen.getByRole('button', { name: '最大化' }))
  expect(native.onResized).not.toHaveBeenCalled()
  expect(native.toggleMaximize).not.toHaveBeenCalled()
})
