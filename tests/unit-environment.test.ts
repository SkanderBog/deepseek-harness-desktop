// @vitest-environment jsdom
import { expect, it } from 'vitest'

it('provides CSSOM identifier escaping for DOM dependencies', () => {
  for (const [value, escaped] of [
    ['', ''],
    ['\0', '\uFFFD'],
    ['\u0001\u007F', '\\1 \\7f '],
    ['0a', '\\30 a'],
    ['-0a', '-\\30 a'],
    ['-', '\\-'],
    ['--a_9', '--a_9'],
    ['a:b c', 'a\\:b\\ c'],
    ['\"\\', '\\\"\\\\'],
    ['中文😀', '中文😀'],
  ] as const) {
    expect(CSS.escape(value)).toBe(escaped)
  }
  expect(window.CSS.escape).toBe(CSS.escape)
})
