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

  it('writes a plain IconUri because the verbatim form is ignored by the toast platform', () => {
    const source = readFileSync(
      new URL('vendor/tauri-plugin-notifications/src/windows.rs', tauriRoot),
      'utf8',
    )
    expect(source).toContain('Some("IconUri"), &plain_icon_path(path)')
    expect(source).not.toContain('Some("IconUri"), &path.to_string_lossy()')
    expect(source).toContain(String.raw`strip_prefix(r"\\?\")`)
    expect(source).toContain(String.raw`strip_prefix(r"\\?\UNC\")`)
  })
})
