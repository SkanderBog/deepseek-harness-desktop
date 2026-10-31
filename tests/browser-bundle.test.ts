import type { PatchOptions } from '@deepseek-ai/cordis-plugin-include'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { Context } from '@deepseek-ai/cordis'
import { applyEntryPatches, entryListSchema } from '@deepseek-ai/cordis-plugin-include'
import Loader from '@deepseek-ai/cordis-plugin-loader'
import { expect, it, onTestFinished } from 'vitest'

const yaml = createRequire(import.meta.resolve('@deepseek-ai/cordis-plugin-include'))('js-yaml') as {
  load: (content: string, options: { schema: typeof entryListSchema }) => PatchOptions[]
}

it.each([undefined, true, false])('desktop bundle activates Browser under a web profile while respecting user override %s', async (override) => {
  const patches = yaml.load(readFileSync(new URL('../packages/builtin-plugin-tauri/cordis.patch.yml', import.meta.url), 'utf8'), { schema: entryListSchema })
  const userPatch = override === undefined ? [] : [{ id: 'ui-sidebar-browser', disabled: override }]
  const entries = applyEntryPatches([{
    id: 'ui-sidebar-browser',
    name: 'cordis:browser',
    disabled: true,
  }], [...patches, ...userPatch], (message) => { throw new Error(message) })
  const ctx = new Context()
  onTestFinished(() => ctx.fiber.dispose())
  let active = 0
  await ctx.plugin(Loader, { baseUrl: import.meta.url }).await()
  ctx.loader.builtins.browser = {
    apply(scope: Context) {
      scope.effect(() => {
        active++
        return () => {
          active--
        }
      }, 'browser test activation')
    },
  }
  ctx.loader.builtins.carrier = { apply() {} }
  await ctx.loader.root.update(entries.map(entry => entry.name === 'dsh-tauri' ? { ...entry, name: 'cordis:carrier' } : entry))
  await ctx.loader.await()
  expect(ctx.loader.resolve('ui-sidebar-browser').disabled).toBe(override === true)
  expect(active).toBe(override === true ? 0 : 1)
  await ctx.fiber.dispose()
  expect(active).toBe(0)
})
