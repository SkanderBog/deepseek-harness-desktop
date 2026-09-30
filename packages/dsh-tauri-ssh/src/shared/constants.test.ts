import { describe, expect, it } from 'vitest'
import { SSH_API_PATH } from '../client/constants/index'
import { machineRowOf } from '../client/store/index'
import { REMOTE_PLUGIN_PROFILE } from '../host/service/sync'
import { DEFAULT_REMOTE_PORT as hostRemotePort, DEFAULT_SSH_PORT as hostSshPort, machineProfileOf } from '../host/storage/index'
import { DEFAULT_REMOTE_PORT, DEFAULT_SSH_PORT, SSH_API_PREFIX } from './constants'

describe('shared SSH defaults', () => {
  it('preserves numeric defaults and legacy host exports', () => {
    expect(DEFAULT_SSH_PORT).toBe(22)
    expect(DEFAULT_REMOTE_PORT).toBe(3080)
    expect(hostSshPort).toBe(22)
    expect(hostRemotePort).toBe(3080)
    expect(REMOTE_PLUGIN_PROFILE).toBe('remote')
    expect(SSH_API_PREFIX).toBe('/api-ssh')
    expect(SSH_API_PATH).toBe('/api-ssh')
  })

  it.each([{}, { port: 'bad', remotePort: null }])('keeps client and stored wire fallback values for %j', (ports) => {
    const wire = { id: 'a', name: 'alpha', host: 'localhost', user: 'ops', ...ports }
    expect(machineRowOf(wire)).toMatchObject({ port: 22, remotePort: 3080 })
    expect(machineProfileOf(wire)).toMatchObject({ port: 22, remotePort: 3080 })
  })

  it('preserves explicitly supplied numeric ports', () => {
    const wire = { id: 'a', name: 'alpha', host: 'localhost', user: 'ops', port: 2222, remotePort: 4000 }
    expect(machineRowOf(wire)).toMatchObject({ port: 2222, remotePort: 4000 })
    expect(machineProfileOf(wire)).toMatchObject({ port: 2222, remotePort: 4000 })
  })
})
