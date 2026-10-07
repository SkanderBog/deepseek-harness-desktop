import { describe, expect, it } from 'vitest'
import { readSource } from './setup/read-source'

const WORKFLOW = '.github/workflows/release-nightly.yml'
const UPLOAD_STEPS = ['Upload assets to the draft release', 'Refresh rolling nightly alias']

function stepBody(source: string, name: string): string {
  const body = source.match(new RegExp(` {6}- name: ${name}\\n([\\s\\S]*?)(?=\\n {6}- |$)`))?.[1]
  expect(body, name).toBeTypeOf('string')
  return body ?? ''
}

describe('nightly release assets', () => {
  it('uploads assets without deleting any by file name', () => {
    const source = readSource(WORKFLOW)
    expect(source).not.toContain('delete-asset')
    for (const name of UPLOAD_STEPS) {
      const body = stepBody(source, name)
      expect(body, name).toContain('gh release upload')
      expect(body, name).toContain('--clobber')
    }
  })

  it('keeps downloaded assets outside the workspace that checkout wipes', () => {
    const lines = readSource(WORKFLOW)
      .split('\n')
      .filter(line => line.includes('release-assets') && !line.trimStart().startsWith('#'))
    expect(lines.length).toBeGreaterThan(3)
    for (const line of lines) {
      expect(line.includes('RUNNER_TEMP') || line.includes('runner.temp'), line).toBe(true)
    }
  })
})
