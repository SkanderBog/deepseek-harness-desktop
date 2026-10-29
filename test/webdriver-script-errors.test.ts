import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { expect, it } from 'vitest'

const template = readFileSync(new URL('../src-tauri/vendor/tauri-plugin-wdio-webdriver/src/platform/script_errors.js.inc', import.meta.url), 'utf8')
const errorKey = '__wdio_script_error_7d36c144'

function wrap(script: string): string {
  return template.replace('__WD_ERROR_KEY__', errorKey).replace('__WD_SCRIPT__', script)
}

it('preserves native script completion and reports exceptions without eval', () => {
  for (const [script, expected] of [
    ['42', 42],
    ['null', null],
    ['undefined', undefined],
    ['var navigation = true; null;', null],
    ['({ success: false, error: "user result" })', { success: false, error: 'user result' }],
    ['({ __wdio_script_error: "user result" })', { __wdio_script_error: 'user result' }],
    ['throw new Error("stale element reference")', { [errorKey]: 'stale element reference' }],
    ['throw new TypeError("ordinary error")', { [errorKey]: 'ordinary error' }],
    ['throw null', { [errorKey]: 'null' }],
  ] as const) {
    expect(runInNewContext(wrap(script), {}, { contextCodeGeneration: { strings: false, wasm: false } })).toEqual(expected)
  }
  const context: Record<string, unknown> = {}
  runInNewContext(wrap('var nativeGlobal = 7'), context)
  expect(context.nativeGlobal).toBe(7)
})
