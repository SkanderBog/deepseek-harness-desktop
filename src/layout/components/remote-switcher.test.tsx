// @vitest-environment jsdom
import type { Remote, SshMachineRow } from '@/hooks/use-remote'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '@/config/query-keys'
import { useRemote } from '@/hooks/use-remote'
import { RemoteSwitcher } from './remote-switcher'

if (typeof globalThis.CSS === 'undefined') {
  Object.assign(globalThis, {
    CSS: {
      escape: (value: string) => value.replace(/[^\w-]/g, c => `\\${c}`),
    },
  })
}

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))
const toastSpy = vi.fn()
const manageSpy = vi.fn()
const invokeSpy = vi.fn<(command: string, args?: { method?: string, payload?: unknown, machineId?: string, url?: string }) => Promise<unknown>>()
vi.mock('@/utils/toast', () => ({
  toast: (...args: unknown[]) => { toastSpy(...args) },
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: Parameters<typeof invokeSpy>) => invokeSpy(...args),
}))

function machineOf(partial: Partial<SshMachineRow>): SshMachineRow {
  return { id: 'm1', name: 'machine', state: 'disconnected', ...partial }
}

function remoteOf(overrides: Partial<Remote> = {}): Remote {
  return {
    machines: [],
    enabled: true,
    available: true,
    activeId: null,
    pendingId: null,
    activeTunnelUrl: '',
    connectTrail: [],
    connectLog: [],
    connectFailed: null,
    connectDismissed: false,
    switchTo: vi.fn(),
    backToLocal: vi.fn(),
    disconnect: vi.fn(),
    dismissConnect: vi.fn(),
    cancelConnect: vi.fn(),
    refresh: vi.fn<Remote['refresh']>(),
    ...overrides,
  }
}

let remote: Remote
let queryClient: QueryClient
let engineMachines: SshMachineRow[] = []
let engineEnabled = true
let engineUnreachable = false
let disconnectSpy = vi.fn()

function HookSwitcher() {
  remote = useRemote()
  return <RemoteSwitcher remote={remote} onManage={manageSpy} />
}

function renderWithHook() {
  queryClient.setQueryData(queryKeys.remoteMachines, { enabled: engineEnabled, machines: engineMachines })
  return render(<QueryClientProvider client={queryClient}><HookSwitcher /></QueryClientProvider>)
}

async function openMenu() {
  fireEvent.click(screen.getByRole('button', { name: 'remote.switcher' }))
  await waitFor(() => {
    expect(screen.getByRole('menu')).toBeTruthy()
  })
  return screen.getByRole('menu')
}

function seedMachines(machines: SshMachineRow[]) {
  engineMachines = machines
  engineEnabled = true
  remote = { ...remote, enabled: true, machines }
}

beforeEach(() => {
  remote = remoteOf()
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  toastSpy.mockClear()
  manageSpy.mockClear()
  invokeSpy.mockReset()
  disconnectSpy = vi.fn()
  engineMachines = []
  engineEnabled = true
  engineUnreachable = false
  invokeSpy.mockImplementation(async (command, args) => {
    if (command === 'remote_open_window')
      return undefined
    if (command !== 'remote')
      throw new Error(`Unexpected native command: ${command}`)
    switch (args?.method) {
      case 'GET /api/desktop/dsh-tauri-ssh/machines':
        if (engineUnreachable)
          throw new TypeError('fetch failed')
        return { enabled: engineEnabled, items: engineMachines, discovered: [] }
      case 'GET /api/desktop/dsh-tauri-ssh/machines/events':
        return { items: [] }
      case 'POST /api/desktop/dsh-tauri-ssh/machines/disconnect':
        disconnectSpy(args.payload)
        return {}
      default:
        throw new Error(`Unexpected remote request: ${args?.method}`)
    }
  })
})

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.restoreAllMocks()
})

describe('remoteSwitcher 渲染', () => {
  it('空态：本地项 + 空态引导 + 管理入口（打开壳层管理面板）', async () => {
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    await openMenu()
    const menu = screen.getByRole('menu')
    expect(within(menu).getByText('remote.local')).toBeTruthy()
    expect(within(menu).getByText('remote.empty')).toBeTruthy()

    fireEvent.click(within(menu).getByText('remote.manage'))
    await waitFor(() => {
      expect(manageSpy).toHaveBeenCalled()
    })
  })

  it('操作区：管理机器与同步到远端同级且各自回调；缺回调时双双置灰', async () => {
    const syncSpy = vi.fn()
    const { unmount } = render(<RemoteSwitcher remote={remote} onManage={manageSpy} onSync={syncSpy} />)
    const menu = await openMenu()
    fireEvent.click(within(menu).getByText('remote.sync'))
    await waitFor(() => {
      expect(syncSpy).toHaveBeenCalled()
    })
    expect(within(menu).getByText('remote.manage')).toBeTruthy()
    unmount()

    render(<RemoteSwitcher remote={remote} />)
    const bare = await openMenu()
    expect(within(bare).getByText('remote.manage').closest('[role="menuitem"]')?.getAttribute('aria-disabled')).toBe('true')
    expect(within(bare).getByText('remote.sync').closest('[role="menuitem"]')?.getAttribute('aria-disabled')).toBe('true')
  })

  it('机器项：状态点语义（标识色优先内联 / 已连接绿 / 重连琥珀 / 放弃红）与状态文案', async () => {
    seedMachines([
      machineOf({ id: 'colored', name: 'colored', color: '#ff00ff', state: 'reconnecting' }),
      machineOf({ id: 'green', name: 'green', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' }),
      machineOf({ id: 'amber', name: 'amber', state: 'connecting' }),
      machineOf({ id: 'red', name: 'red', state: 'given-up', lastError: 'connect failed after 3 attempt(s): refused' }),
    ])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    await openMenu()

    const colored = screen.getByText('colored').closest('[role="menuitem"]')?.querySelector('[class*="rounded-full"]')
    expect(colored?.getAttribute('style')).toContain('rgb(255, 0, 255)')

    const green = screen.getByText('green').closest('[role="menuitem"]')?.querySelector('[class*="rounded-full"]')
    expect(green?.className).toContain('bg-success')
    expect(screen.getByText('remote.state.connected')).toBeTruthy()

    const amber = screen.getByText('amber').closest('[role="menuitem"]')?.querySelector('[class*="rounded-full"]')
    expect(amber?.className).toContain('bg-warning')

    const red = screen.getByText('red').closest('[role="menuitem"]')?.querySelector('[class*="rounded-full"]')
    expect(red?.className).toContain('bg-danger')
    expect(screen.getByText('red').closest('[title]')?.getAttribute('title')).toContain('refused')
  })

  it('触发按钮显示活动机器名与标识色点；无活动时显示本地', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', color: '#123456', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' })])
    remote = { ...remote, activeId: 'm1', activeTunnelUrl: 'http://127.0.0.1:4001' }
    const { rerender } = render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const trigger = screen.getByRole('button', { name: 'remote.switcher' })
    expect(trigger.textContent).toContain('alpha')
    expect(trigger.querySelector('span[class*="rounded-full"]')?.getAttribute('style')).toContain('rgb(18, 52, 86)')

    remote = { ...remote, activeId: null, activeTunnelUrl: '' }
    rerender(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    expect(screen.getByRole('button', { name: 'remote.switcher' }).textContent).toContain('remote.local')
  })
})

describe('remoteSwitcher 交互与降级', () => {
  it('未启用（或插件未加载）时壳层不渲染「本地」控件；启用后随下一轮轮询出现', async () => {
    engineEnabled = false
    renderWithHook()
    await vi.waitFor(() => {
      expect(invokeSpy).toHaveBeenCalledWith('remote', { method: 'GET /api/desktop/dsh-tauri-ssh/machines', payload: null })
      expect(remote.available).toBe(true)
      expect(queryClient.getQueryState(queryKeys.remoteMachines)?.fetchStatus).toBe('idle')
    })
    expect(screen.queryByRole('button', { name: 'remote.switcher' })).toBeNull()

    engineEnabled = true
    window.dispatchEvent(new Event('focus'))
    await vi.waitFor(() => {
      expect(screen.getByRole('button', { name: 'remote.switcher' })).toBeTruthy()
    })
  })

  it('点击已连接机器项：切换视图（activeId/隧道 URL）', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' })])
    renderWithHook()
    const menu = await openMenu()
    fireEvent.click(within(menu).getByText('alpha'))
    await waitFor(() => {
      expect(remote.activeId).toBe('m1')
      expect(remote.activeTunnelUrl).toBe('http://127.0.0.1:4001')
    })
  })

  it('点击本地项：回本地并撤销挂起切换', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', state: 'connecting' })])
    remote = { ...remote, activeId: 'm1', activeTunnelUrl: 'http://127.0.0.1:4001', pendingId: 'm1' }
    const { rerender } = render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    fireEvent.click(within(menu).getByText('remote.local'))
    await waitFor(() => expect(remote.backToLocal).toHaveBeenCalledOnce())
    remote = { ...remote, activeId: null, activeTunnelUrl: '', pendingId: null }
    rerender(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    await waitFor(() => {
      expect(remote.activeId).toBeNull()
      expect(remote.activeTunnelUrl).toBe('')
      expect(remote.pendingId).toBeNull()
    })
    expect(screen.getByRole('button', { name: 'remote.switcher' }).textContent).toContain('remote.local')
  })

  it('本地实例不可达：降级提示 + 远端项禁用（不弹错误风暴），恢复后自动复原', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' })])
    engineUnreachable = true
    renderWithHook()
    await vi.waitFor(() => {
      expect(remote.available).toBe(false)
    })
    const menu = await openMenu()
    expect(within(menu).getByText('remote.degraded')).toBeTruthy()
    const item = within(menu).getByText('alpha').closest('[role="menuitem"]')
    expect(item?.getAttribute('aria-disabled')).toBe('true')

    engineUnreachable = false
    window.dispatchEvent(new Event('focus'))
    await vi.waitFor(() => {
      expect(remote.available).toBe(true)
    })
    await vi.waitFor(() => {
      expect(screen.queryByText('remote.degraded')).toBeNull()
    })
    expect(toastSpy).not.toHaveBeenCalled()
  })
})

describe('remoteSwitcher 增强（4.4）', () => {
  it('活动机器：状态点强制绿色 + 底部「断开当前连接」一键断开并回本地', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' })])
    renderWithHook()
    act(() => remote.switchTo('m1'))
    const menu = await openMenu()

    const dot = within(menu).getByText('alpha').closest('[role="menuitem"]')?.querySelector('[class*="rounded-full"]')
    expect(dot?.className).toContain('bg-success')

    fireEvent.click(within(menu).getByText('remote.disconnect_active'))
    await waitFor(() => {
      expect(disconnectSpy).toHaveBeenCalledWith({ machineId: 'm1' })
    })
    expect(remote.activeId).toBeNull()
    expect(remote.activeTunnelUrl).toBe('')
  })

  it('无活动机器时不出现断开项', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' })])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    expect(within(menu).queryByText('remote.disconnect_active')).toBeNull()
  })

  it('重连机器显示重试倒计时；已知凭据类型缀在状态后', async () => {
    seedMachines([
      machineOf({ id: 'm1', name: 'alpha', state: 'reconnecting', nextRetryAt: Date.now() + 42_000 }),
      machineOf({ id: 'm2', name: 'beta', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4002', authMethod: 'key' }),
    ])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    const alpha = within(menu).getByText('alpha').closest('[role="menuitem"]')
    expect(alpha?.textContent).toContain('remote.retry_in')
    const beta = within(menu).getByText('beta').closest('[role="menuitem"]')
    expect(beta?.textContent).toContain('remote.auth.key')
  })
})

describe('remoteSwitcher 行内双动作（当前窗口 vs 新窗口）', () => {
  it('新窗口按钮常驻所有机器行：点击只发 remote_open_window，不触发切换', async () => {
    seedMachines([
      machineOf({ id: 'm1', name: 'alpha', state: 'connected', tunnelBaseUrl: 'http://127.0.0.1:4001' }),
      machineOf({ id: 'm2', name: 'beta', state: 'disconnected' }),
    ])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    const buttons = within(menu).getAllByRole('button', { name: 'remote.open_new_window' })
    expect(buttons.length).toBe(2)
    fireEvent.click(buttons[0]!)
    await waitFor(() => {
      expect(invokeSpy).toHaveBeenCalledWith('remote_open_window', { machineId: 'm1', url: 'http://127.0.0.1:4001' })
    })
    expect(remote.activeId).toBeNull()
    expect(remote.pendingId).toBeNull()
    expect(remote.switchTo).not.toHaveBeenCalled()
  })

  it('未连接机器的新窗口按钮：url 置空（窗口内启动连接流程）', async () => {
    seedMachines([machineOf({ id: 'm2', name: 'beta', state: 'disconnected' })])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    const button = within(menu).getByRole('button', { name: 'remote.open_new_window' })
    fireEvent.click(button)
    await waitFor(() => {
      expect(invokeSpy).toHaveBeenCalledWith('remote_open_window', { machineId: 'm2', url: '' })
    })
    expect(remote.activeId).toBeNull()
    expect(remote.switchTo).not.toHaveBeenCalled()
  })

  it('机器行显示 user@host:port 副标题', async () => {
    seedMachines([machineOf({ id: 'm1', name: 'alpha', host: '10.1.1.1', port: 22, user: 'root', state: 'disconnected' })])
    render(<RemoteSwitcher remote={remote} onManage={manageSpy} />)
    const menu = await openMenu()
    expect(within(menu).getByText('root@10.1.1.1:22')).toBeTruthy()
  })
})
