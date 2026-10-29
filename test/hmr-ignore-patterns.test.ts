import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { posix, win32 } from 'node:path'
import { describe, expect, it } from 'vitest'

const require = createRequire(import.meta.url)
const viteRequire = createRequire(require.resolve('vite'))
const picomatch = createRequire(viteRequire.resolve('tinyglobby'))('picomatch') as (patterns: string[]) => (path: string) => boolean
const source = readFileSync(new URL('../src-tauri/src/service/plugin/hmr.rs', import.meta.url), 'utf8')
const body = source.match(/fn ignore_patterns\(\)[\s\S]*?vec!\[([\s\S]*?)\n\}/)![1]
const patterns = [...body.matchAll(/("(?:[^"\\]|\\.)*")\.to_string\(\)/g)].map(match => JSON.parse(match[1]) as string)
const ignored = picomatch(patterns)
const golden = source.match(/let golden = \[([\s\S]*?)\n {8}\]/)![1]
const rootedPatterns = [...golden.matchAll(/("(?:[^"\\]|\\.)*"),/g)]
  .map(match => JSON.parse(match[1]) as string)
  .filter(line => line.startsWith('      - \''))
  .map(line => line.slice(9, -1).replace(/''/g, '\''))
const rootedIgnored = picomatch(rootedPatterns)

for (const paths of [posix, win32]) {
  describe(`${paths.sep} HMR ignore rules`, () => {
    it('prunes dependency and hidden subtrees at the root and every depth', () => {
      expect(patterns.length).toBeGreaterThan(0)
      for (const parent of ['', 'plugin', paths.join('plugin', 'nested')]) {
        for (const name of ['node_modules', '.git', '.worktrees', '.cache']) {
          for (const child of ['', 'file.js', paths.join('nested', 'file.js')]) {
            const path = paths.relative(paths.resolve('hmr-root'), paths.resolve('hmr-root', parent, name, child))
            expect(ignored(path), `必须忽略 ${path}`).toBe(true)
          }
        }
      }
    })

    it('keeps explicit roots beneath hidden ancestors watchable', () => {
      for (const root of [paths.join('repo', '.worktrees', 'feature'), paths.join('other', 'sub')]) {
        for (const child of ['', 'src', paths.join('src', 'index.ts')]) {
          const path = paths.relative(paths.resolve('hmr-root'), paths.resolve('hmr-root', root, child))
          expect(rootedIgnored(path), `必须监听显式根目录 ${path}`).toBe(false)
        }
        for (const parent of ['', 'nested']) {
          for (const name of ['node_modules', '.git', '.worktrees', '.cache']) {
            for (const child of ['', 'file.js', paths.join('nested', 'file.js')]) {
              const path = paths.relative(paths.resolve('hmr-root'), paths.resolve('hmr-root', root, parent, name, child))
              expect(rootedIgnored(path), `必须忽略根目录内的 ${path}`).toBe(true)
            }
          }
        }
      }
    })

    it('keeps plugin source and similarly named directories watchable', () => {
      for (const path of ['index.ts', 'package.json', paths.join('plugin', 'src', 'index.ts'), paths.join('plugin', 'node_modules-extra', 'file.ts'), paths.join('plugin', 'git', 'file.ts')])
        expect(ignored(path), `必须监听 ${path}`).toBe(false)
    })
  })
}
