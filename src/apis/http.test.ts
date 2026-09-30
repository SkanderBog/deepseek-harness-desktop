import { beforeEach, describe, expect, it, vi } from 'vitest'

const { tauriFetch } = vi.hoisted(() => ({ tauriFetch: vi.fn() }))

vi.mock('@tauri-apps/plugin-http', () => ({ fetch: tauriFetch }))
vi.mock('@/store/modules/harness', () => ({ harness: { $state: { serviceUrl: 'http://127.0.0.1:3081' } } }))

const { SshApiHttpError, ofetch } = await import('./http')

function answer(status: number, body = ''): void {
  tauriFetch.mockResolvedValueOnce({ ok: status < 400, status, text: async () => body })
}

beforeEach(() => {
  tauriFetch.mockReset()
})

describe('ofetch', () => {
  it('拼出 serviceUrl + baseURL + path + query，走内核 fetch', async () => {
    answer(200, JSON.stringify({ enabled: true }))

    await expect(ofetch('/machines', {
      baseURL: '/api/desktop/dsh-tauri-ssh',
      params: { machineId: 'm1', sinceSeq: 3, dropped: undefined },
    })).resolves.toEqual({ enabled: true })

    expect(tauriFetch).toHaveBeenCalledWith(
      'http://127.0.0.1:3081/api/desktop/dsh-tauri-ssh/machines?machineId=m1&sinceSeq=3',
      { method: 'GET' },
    )
  })

  it('有 body 时序列化并声明 JSON', async () => {
    answer(200, JSON.stringify({ tunnelBaseUrl: 'http://127.0.0.1:4001' }))

    await ofetch('/machines/connect', { baseURL: '/api/desktop/dsh-tauri-ssh', method: 'post', body: { machineId: 'm1' } })

    expect(tauriFetch).toHaveBeenCalledWith(
      'http://127.0.0.1:3081/api/desktop/dsh-tauri-ssh/machines/connect',
      { method: 'POST', body: JSON.stringify({ machineId: 'm1' }), headers: { 'content-type': 'application/json' } },
    )
  })

  it('404/405 是「本地实例没有这个 API」，用稳定错误码区分于不可达', async () => {
    answer(405)

    const error = await ofetch('/machines').catch((err: unknown) => err)

    expect(error).toBeInstanceOf(SshApiHttpError)
    expect(error).toMatchObject({ status: 405, message: 'SSH_API_HTTP_405' })
  })

  it('其余非 2xx 透出服务端消息，无消息则退化为状态码', async () => {
    answer(400, JSON.stringify({ error: '缺少 machineId' }))
    await expect(ofetch('/machines/connect', { method: 'post' })).rejects.toThrow('缺少 machineId')

    answer(503, '<html>upstream</html>')
    await expect(ofetch('/machines')).rejects.toThrow('SSH_API_HTTP_503')
  })

  it('2xx 但正文不可解析时返回 undefined，而不是解析异常', async () => {
    answer(200, '')
    await expect(ofetch('/settings')).resolves.toBeUndefined()

    answer(200, 'not json')
    await expect(ofetch('/settings')).resolves.toBeUndefined()
  })
})
