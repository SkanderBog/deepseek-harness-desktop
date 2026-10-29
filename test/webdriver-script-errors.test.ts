import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { expect, it } from 'vitest'

const template = readFileSync(new URL('../src-tauri/vendor/tauri-plugin-wdio-webdriver/src/platform/script_errors.js.inc', import.meta.url), 'utf8')

it('preserves native script completion and reports exceptions without eval', () => {
  for (const [script, expected] of [
    ['42', 42],
    ['null', null],
    ['undefined', undefined],
    ['var navigation = true; null;', null],
    ['({ success: false, error: "user result" })', { success: false, error: 'user result' }],
    ['throw new Error("stale element reference")', { __wdio_script_error: 'stale element reference' }],
    ['throw new TypeError("ordinary error")', { __wdio_script_error: 'ordinary error' }],
    ['throw null', { __wdio_script_error: 'null' }],
  ] as const) {
    expect(runInNewContext(template.replace('__WD_SCRIPT__', script), {}, { contextCodeGeneration: { strings: false, wasm: false } })).toEqual(expected)
  }
  const context: Record<string, unknown> = {}
  runInNewContext(template.replace('__WD_SCRIPT__', 'var nativeGlobal = 7'), context)
  expect(context.nativeGlobal).toBe(7)
})
