import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const tauriRoot = new URL('../src-tauri/', import.meta.url)
const config = JSON.parse(readFileSync(new URL('tauri.conf.json', tauriRoot), 'utf8'))

describe('windows notification app identity icon', () => {
  it('configures a bundled PNG rather than an executable icon resource', () => {
    expect(config.plugins.notifications.windows.iconPath).toBe('icons/32x32.png')
    expect(config.bundle.resources).toContain('icons/32x32.png')
    const icon = readFileSync(fileURLToPath(new URL('icons/32x32.png', tauriRoot)))
    expect([...icon.subarray(0, 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10])
    expect(icon.readUInt32BE(16)).toBe(32)
    expect(icon.readUInt32BE(20)).toBe(32)
  })
})
