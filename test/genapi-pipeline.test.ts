import type { ApiPipeline } from '@genapi/shared'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { config } from '@genapi/pipeline'
import { context, inject } from '@genapi/shared'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { pluginPipeline } from '../genapi.pipeline'

const stages = vi.hoisted(() => ({ original: undefined as unknown as (value: ApiPipeline.ConfigRead) => ApiPipeline.ConfigRead }))

vi.mock('@genapi/pipeline', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@genapi/pipeline')>()
  return {
    ...actual,
    default: (...steps: Parameters<typeof actual.default>) => {
      stages.original = steps[1] as typeof stages.original
      return actual.default(...steps)
    },
    dest: async () => {},
  }
})

const roots: string[] = []

afterEach(() => {
  for (const key of Object.keys(context))
    delete context[key]
  for (const root of roots.splice(0)) {
    if (dirname(root) !== resolve(tmpdir()) || !root.startsWith(join(resolve(tmpdir()), 'genapi-test-')))
      throw new Error(`Unexpected fixture root: ${root}`)
    rmSync(root, { recursive: true, force: true })
  }
})

function fixture(files: Record<string, string>): ApiPipeline.Config {
  const root = mkdtempSync(join(resolve(tmpdir()), 'genapi-test-'))
  roots.push(root)
  const routes = join(root, 'fixture', 'src/host/routes')
  mkdirSync(routes, { recursive: true })
  for (const [file, source] of Object.entries(files)) {
    const target = join(root, 'fixture', 'src', file)
    mkdirSync(dirname(target), { recursive: true })
    writeFileSync(target, source)
  }
  return { input: routes, output: { main: 'fixture/api.ts', type: 'fixture/api.type.ts' }, meta: { baseURL: '"/fixture"', import: { http: 'dsh-tauri/client' } } }
}

function scan(files: Record<string, string>) {
  return stages.original(config(fixture(files)))
}

function response(annotation: string) {
  return scan({ 'host/routes/get.ts': `export default defineEventHandler((event): ${annotation} => undefined)` }).source
}

describe('generator scanner compatibility', () => {
  it.each([
    ['string | null', { type: 'string' }],
    ['Array<string> | number', { type: 'number' }],
    ['{ value: "a|b;c"; nested: { flag: boolean }; list: [number, string] }', { type: 'object', properties: { value: { type: 'string', enum: ['a|b;c'] }, nested: { type: 'object', properties: { flag: { type: 'boolean' } }, required: ['flag'] }, list: { type: 'array', items: { type: 'number' } } }, required: ['value', 'nested', 'list'] }],
    ['{ label: `a}|b;c`; next: number }', { type: 'object', properties: { label: { type: 'string' }, next: { type: 'number' } }, required: ['label', 'next'] }],
    ['{ callback: () => void; next: number }', { type: 'object', properties: { callback: { type: 'string' } }, required: ['callback'] }],
    ['Promise<{ callback: () => void; next: number }>', { type: 'object', properties: { callback: { type: 'string' } }, required: ['callback'] }],
  ])('return annotation %s preserves its schema', (annotation, schema) => {
    expect(response(annotation)).toEqual({ swagger: '2.0', info: { title: 'fixture', version: '0.0.0' }, paths: { '': { get: { operationId: 'get', parameters: [], responses: { 200: { description: 'GET /', schema } } } } }, definitions: {} })
  })

  it('member newlines continue a leading union but split the next field', () => {
    const result = response('{ mode: "a"\n | "b"\n count?: number, done: boolean; }')
    expect(result.paths[''].get.responses[200].schema).toEqual({ type: 'object', properties: { mode: { type: 'string', enum: ['a', 'b'] }, count: { type: 'number' }, done: { type: 'boolean' } }, required: ['mode', 'done'] })
  })

  it('escaped quotes do not expose member delimiters', () => {
    const result = response(String.raw`{ first: 'a\';|b'; second: "c\",;d"; third: number }`)
    expect(result.paths[''].get.responses[200].schema).toEqual({ type: 'object', properties: { first: { type: 'string' }, second: { type: 'string' }, third: { type: 'number' } }, required: ['first', 'second', 'third'] })
  })

  it('nested generic arguments preserve value schemas and quoted angles', () => {
    const result = response('{ values: Record<"a>b", Array<{ count: number }>>; flags: Map<string, boolean>; names: ReadonlyArray<string> }')
    expect(result.paths[''].get.responses[200].schema).toEqual({ type: 'object', properties: { values: { type: 'object', additionalProperties: { type: 'array', items: { type: 'object', properties: { count: { type: 'number' } }, required: ['count'] } } }, flags: { type: 'object', additionalProperties: { type: 'boolean' } }, names: { type: 'array', items: { type: 'string' } } }, required: ['values', 'flags', 'names'] })
  })

  it('angle-only request scanning retains arrow truncation', () => {
    const result = scan({ 'host/routes/post.ts': 'export default defineEventHandler((event) => { const body = readBody<{ callback: () => void; next: number }>(event) })' })
    expect(result.source.paths[''].post.parameters).toEqual([{ name: 'body', in: 'body', required: true, schema: { $ref: '#/definitions/postBody' } }])
    expect(result.source.definitions).toEqual({ postBody: { type: 'object', properties: { callback: { type: 'string' } }, required: ['callback'] } })
  })

  it('generic response arguments take precedence over arrow annotations', () => {
    const result = scan({ 'host/routes/get.ts': 'export default defineEventHandler<EventHandlerRequest, Promise<{ value: "a>b" }>>((event): number => 1)' })
    expect(result.source.paths[''].get.responses).toEqual({ 200: { description: 'GET /', schema: { type: 'object', properties: { value: { type: 'string', enum: ['a>b'] } }, required: ['value'] } } })
  })

  it('balanced parameter scanning ignores angles and quoted closing brackets', () => {
    const result = scan({ 'host/routes/get.ts': 'export default defineEventHandler(async (event = { value: ")]}>", nested: [1] }): Promise<number[]> => [])' })
    expect(result.source.paths[''].get.responses).toEqual({ 200: { description: 'GET /', schema: { type: 'array', items: { type: 'number' } } } })
  })

  it('missing balanced closing brackets omit response schemas', () => {
    const result = scan({ 'host/routes/get.ts': 'export default defineEventHandler((event: { value: string }: number => 1' })
    expect(result.source.paths[''].get.responses).toEqual({ 200: { description: 'GET /' } })
  })

  it('missing generic closing angles preserve fallback request schemas', () => {
    const result = scan({ 'host/routes/post.ts': 'export default defineEventHandler((event) => readBody<{ value: string }(event))' })
    expect(result.source.paths[''].post.parameters).toEqual([])
  })

  it('multiline aliases preserve union and intersection continuations verbatim', () => {
    const result = scan({ 'host/types/contracts.ts': 'export type Result = { kind: "ok" }\n | { kind: "failed" } &\n { reason: string }\nexport type Unused = number\n', 'host/routes/get.ts': 'export default defineEventHandler((event): Result => undefined)' })
    expect(result.graphs.scopes.type.typings).toEqual([{ name: 'Result', value: '{ kind: "ok" }\n | { kind: "failed" } &\n { reason: string }', export: true }])
    expect(result.source.definitions).toEqual({})
  })

  it('named query inheritance and aliased imports retain field contracts', () => {
    const result = scan({ 'shared/contracts.types.ts': 'export interface Base { id: string }\nexport interface Query extends Base { mode?: "a" | "b" }\n', 'host/routes/get.ts': 'import type { Query as LocalQuery, type Base } from "../../shared/contracts.types"\nexport default defineEventHandler((event): number => { getQuery<LocalQuery>(event) })' })
    expect(result.source.paths[''].get.parameters).toEqual([{ type: 'string', name: 'id', in: 'query', required: true }, { type: 'string', enum: ['a', 'b'], name: 'mode', in: 'query', required: false }])
  })

  it('successive runs clear named typing state', () => {
    scan({ 'host/routes/get.ts': 'type First = string\nexport default defineEventHandler((event): First => "")' })
    const result = scan({ 'host/routes/get.ts': 'export default defineEventHandler((event): boolean => true)' })
    expect(result.graphs.scopes.type.typings).toEqual([])
    expect(result.source.paths[''].get.responses).toEqual({ 200: { description: 'GET /', schema: { type: 'boolean' } } })
  })

  it('missing route directories retain the generator rejection', () => {
    const options = fixture({})
    options.input = join(String(options.input), 'missing')
    expect(() => stages.original(config(options))).toThrow(`genapi: 路由目录不存在 ${options.input}，请检查 genapi.config.ts 的 input`)
  })

  it('the real compiler emits the literal client API and named type contract', async () => {
    await pluginPipeline(fixture({ 'host/routes/item/post.ts': 'export type Result = { kind: "ok" | "failed" }\nexport default defineEventHandler(async (event): Promise<Result> => { const body = await readBody<{ name: string; count?: number }>(event); return { kind: "ok" } })' }))
    const result = inject().configRead!
    expect(result.outputs.find(output => output.type === 'main')!.code).toBe('/*\n * @title fixture\n * @swagger 2.0\n * @version 0.0.0\n */\n\nimport type { FetchOptions } from "dsh-tauri/client";\nimport { ofetch } from "dsh-tauri/client";\nimport type * as Types from "./api.type";\n\nexport const baseURL = "/fixture";\n\n/** @method post */\nexport function postItem(body: Types.PostItemBody, options?: FetchOptions) {\n  return ofetch<Types.Result>("/item", { baseURL, method: "post", body, ...options });\n}\n')
    expect(result.outputs.find(output => output.type === 'type')!.code).toBe('export type Result = { kind: "ok" | "failed" };\n\nexport interface PostItemBody {\n  name: string;\n  count?: number;\n}\n')
  })
})
