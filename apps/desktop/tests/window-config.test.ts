import { expect, it } from 'vitest'
import config from '../src-tauri/tauri.conf.json'
import macosConfig from '../src-tauri/tauri.macos.conf.json'

it.each([
  ['默认', config],
  ['macOS', macosConfig],
])('%s 主窗口配置保留 860×620 的最小内容尺寸', (_, config) => {
  expect(config.app.windows[0]).toMatchObject({ minWidth: 860, minHeight: 620 })
})
