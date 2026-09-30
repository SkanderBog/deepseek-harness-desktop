import type { ApiPipeline, StatementField } from '@genapi/shared'
import type { Dirent } from 'node:fs'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { basename, dirname, relative, resolve, sep } from 'node:path'
import process from 'node:process'
import pipeline, { compiler, config, dest, generate } from '@genapi/pipeline'
import { parser } from '@genapi/presets/swag-ofetch-ts'

/**
 * genapi 管道：唯一自定义步骤是 original —— 把插件 `src/host/routes/**` 的 handler 源码推导成 Swagger 2。
 * `input` 即路由目录，文件路径 = URL 路径 = 函数名：`routes/session/archive/get.ts` → GET `/session/archive` → `getSessionArchive`；
 * 请求参数取自 `getQuery(event) as {...}` / `readBody<T>(event)`，响应类型取自 `defineEventHandler(...)` 的返回注解。
 * baseURL 与 fetch 导入一律来自 defineConfig 的 `meta`，管道内不做任何自定义解析。
 */
const RE_HANDLER = /defineEventHandler\s*\(/
const RE_SKIP_FILE = /\.(?:test|spec)\.tsx?$|\.d\.ts$/
const RE_TS_FILE = /\.tsx?$/
const METHOD_FILES = new Set(['get', 'head', 'post', 'put', 'patch', 'delete', 'options'])
const reservedNames = new Set(['delete', 'new', 'await', 'yield', 'in', 'do', 'if', 'for', 'class', 'function', 'return', 'void', 'typeof', 'instanceof', 'switch', 'case', 'default', 'this', 'super', 'import', 'export', 'with'])
const RE_TYPES_FILE = /(?:\.types\.tsx?|[\\/]types[\\/][^\\/]+\.tsx?)$/
const RE_OUTPUT_DIR = /[\\/]client[\\/]apis[\\/]/
const NULLABLE_MEMBERS = new Set(['null', 'undefined'])
/** 具名类型（原样 TS 别名）：由 original 汇总后写回类型输出的 typings。 */
const TYPINGS = new Map<string, string>()

interface Schema {
  $ref?: string
  type?: string
  enum?: string[]
  items?: Schema
  properties?: Record<string, Schema>
  required?: string[]
  additionalProperties?: Schema
}

interface Declaration {
  name: string
  file: string
  kind: 'interface' | 'alias'
  body: string
  bases?: string[]
}

interface ImportBinding {
  file: string
  names: Map<string, string>
}

interface TypeIndex {
  declarations: Map<string, Declaration[]>
  imports: Map<string, Map<string, ImportBinding>>
}

interface HandlerParameter {
  name: string
  location: 'query' | 'body'
  required: boolean
  schema: Schema
}

type QueryParameter = Omit<Schema, 'required'> & {
  name: string
  in: 'query'
  required: boolean
}

interface BodyParameter {
  name: string
  in: 'body'
  required: boolean
  schema: Schema
}

type OperationParameter = QueryParameter | BodyParameter

interface Operation {
  operationId: string
  parameters: OperationParameter[]
  responses: Record<string, { description: string, schema?: Schema }>
}

interface RouteFile {
  segments: string[]
  method: string
  file: string
}

function toPosix(value: string): string {
  return value.split(sep).join('/')
}

function walkFiles(root: string, predicate: (file: string) => boolean): string[] {
  const found: string[] = []
  const stack = [root]
  while (stack.length > 0) {
    const current = stack.pop()!
    let entries: Dirent[]
    try {
      entries = readdirSync(current, { withFileTypes: true })
    }
    catch {
      continue
    }
    for (const entry of entries) {
      if (entry.name.startsWith('.') || entry.name === 'node_modules')
        continue
      const full = resolve(current, entry.name)
      if (entry.isDirectory()) {
        stack.push(full)
        continue
      }
      if (predicate(full))
        found.push(full)
    }
  }
  return found.sort()
}

function toIdentifier(segments: string[]): string {
  return segments
    .flatMap(segment => segment.split(/[^A-Z0-9]+/i))
    .filter(Boolean)
    .map(segment => segment[0].toUpperCase() + segment.slice(1))
    .join('')
}

function operationName(method: string, segments: string[]): string {
  return reservedNames.has(method)
    ? `${method}${toIdentifier(segments)}Root`
    : `${method}${toIdentifier(segments)}`
}

function* scan(source: string, start: number, options: { open: string, close: string, stopOnClose?: boolean, stopOn?: (index: number) => boolean }): Generator<number, undefined> {
  let depth = 0
  let quote: string | undefined
  for (let index = start; index < source.length; index++) {
    const char = source[index]
    if (quote) {
      if (char === '\\') {
        index++
        continue
      }
      if (char === quote)
        quote = undefined
      continue
    }
    if (char === '\'' || char === '"' || char === '`') {
      quote = char
      continue
    }
    if (options.open.includes(char)) {
      depth++
      continue
    }
    if (options.close.includes(char)) {
      depth--
      if (depth === 0 && options.stopOnClose)
        yield index
      continue
    }
    if (depth === 0 && options.stopOn?.(index))
      yield index
  }
}

function splitTopLevel(input: string): string[] {
  const parts: string[] = []
  let start = 0
  for (const index of scan(input, 0, { open: '{[(<', close: '}])>', stopOn: index => input[index] === ';' || input[index] === '|' })) {
    parts.push(input.slice(start, index))
    start = index + 1
  }
  parts.push(input.slice(start))
  return parts.map(part => part.trim()).filter(Boolean)
}

function unwrap(input: string): string {
  const trimmed = input.trim()
  if (trimmed.startsWith('{'))
    return (trimmed.endsWith('}') ? trimmed.slice(1, -1) : trimmed.slice(1)).trim()
  if (trimmed.startsWith('('))
    return (trimmed.endsWith(')') ? trimmed.slice(1, -1) : trimmed.slice(1)).trim()
  return trimmed
}

function splitMembers(input: string): string[] {
  const parts: string[] = []
  let start = 0
  for (const index of scan(input, 0, { open: '{[(<', close: '}])>', stopOn: index => input[index] === ';' || input[index] === ',' || (input[index] === '\n' && !/^\s*\|/.test(input.slice(index + 1))) })) {
    parts.push(input.slice(start, index))
    start = index + 1
  }
  const tail = input.slice(start)
  if (tail.trim().length > 0)
    parts.push(tail)
  return parts.map(part => part.trim().replace(/^[;,]/, '').replace(/[;,]$/, '').trim()).filter(Boolean)
}

function parseFields(body: string): StatementField[] {
  const fields: StatementField[] = []
  for (const member of splitMembers(unwrap(body))) {
    const match = /^(?:readonly\s+)?(?:\[[^\]]+\]|['"]?(\w+)['"]?)(\??)\s*:\s*(\S[\s\S]*)$/.exec(member)
    if (!match)
      continue
    fields.push({ name: match[1], type: match[3].replace(/;$/, '').trim(), required: match[2] !== '?' })
  }
  return fields
}

function genericArgs(input: string): string[] {
  const start = input.indexOf('<')
  if (start < 0)
    return []
  const index = scan(input, start, { open: '<', close: '>', stopOnClose: true }).next().value
  return index === undefined ? [] : splitMembers(input.slice(start + 1, index))
}

function genericOf(source: string, name: string): string | undefined {
  const found = new RegExp(`\\b${name}\\s*<`).exec(source)
  if (!found)
    return undefined
  const start = found.index + found[0].length
  const index = scan(source, start - 1, { open: '<', close: '>', stopOnClose: true }).next().value
  return index === undefined ? undefined : source.slice(start, index).trim()
}

function sliceBalanced(source: string, openIndex: number): string | undefined {
  const index = scan(source, openIndex, { open: '{[(', close: '}])', stopOnClose: true }).next().value
  return index === undefined ? undefined : source.slice(openIndex + 1, index)
}

function sliceDeclaration(source: string, start: number): string | undefined {
  for (const index of scan(source, start, { open: '{[(<', close: '}])>', stopOn: index => source[index] === '\n' })) {
    const collected = source.slice(start, index).trimEnd()
    const next = source.slice(index + 1).replace(/^\s+/, '')[0]
    if (!/[|&]$/.test(collected) && next !== '|' && next !== '&')
      return source.slice(start, index).trim()
  }
  return source.slice(start).trim()
}

function extractDeclarations(source: string, file: string, target: Map<string, Declaration[]>) {
  const push = (declaration: Declaration) => {
    const list = target.get(declaration.name) ?? []
    list.push(declaration)
    target.set(declaration.name, list)
  }
  for (const match of source.matchAll(/(?:export\s+)?(?:declare\s+)?interface\s+([\w$]+)/g)) {
    const headStart = match.index + match[0].length
    const open = source.indexOf('{', headStart)
    if (open < 0)
      continue
    const head = source.slice(headStart, open)
    if (head.length > 200 || head.includes(';') || head.includes('}'))
      continue
    const body = sliceBalanced(source, open)
    if (body === undefined)
      continue
    const extendsAt = head.indexOf('extends')
    const clause = extendsAt >= 0 ? head.slice(extendsAt + 'extends'.length).trim() : ''
    const bases = clause.split(',').map(base => base.trim()).filter(Boolean)
    push({ name: match[1], file, kind: 'interface', body, ...bases.length > 0 ? { bases } : {} })
  }
  for (const match of source.matchAll(/(?:export\s+)?(?:declare\s+)?type\s+(\w+)\s*=\s*/g)) {
    const body = sliceDeclaration(source, match.index + match[0].length)
    if (body !== undefined)
      push({ name: match[1], file, kind: 'alias', body })
  }
}

function importNames(clause: string): Map<string, string> {
  const names = new Map<string, string>()
  for (const raw of splitMembers(clause)) {
    const part = raw.startsWith('type ') ? raw.slice(5).trim() : raw
    const aliased = /^([A-Za-z_$][\w$]*)\s+as\s+([A-Za-z_$][\w$]*)$/.exec(part)
    if (aliased) {
      names.set(aliased[2], aliased[1])
      continue
    }
    if (/^[A-Z_$][\w$]*$/i.test(part))
      names.set(part, part)
  }
  return names
}

function resolveSpecifier(file: string, specifier: string): string | undefined {
  const base = resolve(dirname(file), specifier)
  for (const candidate of [`${base}.ts`, `${base}.tsx`, resolve(base, 'index.ts'), resolve(base, 'index.tsx')]) {
    if (existsSync(candidate))
      return candidate
  }
  return undefined
}

function collectImports(source: string, file: string): Map<string, ImportBinding> {
  const map = new Map<string, ImportBinding>()
  for (const statement of source.matchAll(/import[^\n]*?from\s*['"][^'"]+['"]/g)) {
    const parts = /\bfrom\s*['"]([^'"]+)['"]\s*$/.exec(statement[0])
    if (!parts)
      continue
    const specifier = parts[1]
    if (!specifier.startsWith('.'))
      continue
    const clause = statement[0].slice('import'.length, statement[0].lastIndexOf('from')).trim().replace(/^type\s+/, '')
    if (clause.startsWith('*'))
      continue
    const defaultMatch = /^\w+\s*,?\s*/.exec(clause)
    const rest = defaultMatch ? clause.slice(defaultMatch[0].length) : clause
    const brace = /\{([\s\S]*)\}/.exec(rest)
    if (!brace)
      continue
    const names = importNames(brace[1])
    if (names.size === 0)
      continue
    const target = resolveSpecifier(file, specifier)
    if (target)
      map.set(specifier, { file: target, names })
  }
  return map
}

function typesRank(file: string): number {
  if (/[\\/]src[\\/]host[\\/]/.test(file))
    return 0
  if (/[\\/]src[\\/]shared[\\/]/.test(file))
    return 1
  if (/[\\/]src[\\/]client[\\/]/.test(file))
    return 3
  return 2
}

function buildTypeIndex(src: string): TypeIndex {
  const index: TypeIndex = { declarations: new Map(), imports: new Map() }
  for (const file of walkFiles(src, candidate => RE_TYPES_FILE.test(candidate) && RE_TS_FILE.test(candidate) && !RE_SKIP_FILE.test(candidate) && !RE_OUTPUT_DIR.test(candidate))) {
    const source = readFileSync(file, 'utf8')
    index.imports.set(file, collectImports(source, file))
    extractDeclarations(source, file, index.declarations)
  }
  return index
}

function lookup(index: TypeIndex, name: string, file: string): Declaration | undefined {
  const scope = index.imports.get(file)
  const entry = scope ? [...scope.values()].find(candidate => candidate.names.has(name)) : undefined
  const original = entry?.names.get(name)
  const imported = original === undefined ? [] : index.declarations.get(original) ?? []
  const preferred = imported.find(candidate => candidate.file === entry?.file) ?? imported[0]
  if (preferred)
    return preferred
  const candidates = index.declarations.get(name) ?? []
  const local = candidates.find(candidate => candidate.file === file)
  if (local)
    return local
  const ranked = [...candidates].sort((a, b) => typesRank(a.file) - typesRank(b.file) || a.file.localeCompare(b.file))
  return ranked[0]
}

function toSchema(type: string, index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): Schema {
  const trimmed = type.trim().replace(/^readonly\s+/, '')
  if (trimmed.length === 0 || NULLABLE_MEMBERS.has(trimmed))
    return { type: 'string' }
  const parts = splitTopLevel(trimmed)
  if (parts.length > 1)
    return unionSchema(parts, index, file, definitions, referenced)
  if (trimmed.startsWith('{'))
    return objectSchema(trimmed, index, file, definitions, referenced)
  if (trimmed.startsWith('[') && trimmed.endsWith(']'))
    return { type: 'array', items: toSchema(splitMembers(trimmed.slice(1, -1))[0] ?? 'string', index, file, definitions, referenced) }
  if (trimmed.startsWith('(') && trimmed.endsWith(')'))
    return toSchema(trimmed.slice(1, -1), index, file, definitions, referenced)
  if (trimmed.includes('=>'))
    return { type: 'string' }
  if (trimmed.endsWith('[]'))
    return { type: 'array', items: toSchema(trimmed.slice(0, -2), index, file, definitions, referenced) }
  const literal = /^['"]([^'"]*)['"]$/.exec(trimmed)
  if (literal)
    return { type: 'string', enum: [literal[1]] }
  if (trimmed === 'string' || trimmed === 'number' || trimmed === 'boolean' || trimmed === 'true' || trimmed === 'false')
    return { type: trimmed === 'true' || trimmed === 'false' ? 'boolean' : trimmed }
  if (trimmed === 'unknown' || trimmed === 'any' || trimmed === 'object' || trimmed.startsWith('typeof '))
    return { type: 'string' }
  const generic = /^([A-Z_$][\w$.]*)\s*</i.exec(trimmed)
  if (generic) {
    const args = genericArgs(trimmed)
    if (generic[1] === 'Record')
      return { type: 'object', additionalProperties: toSchema(args[1] ?? 'string', index, file, definitions, referenced) }
    if (generic[1] === 'Map')
      return { type: 'object', additionalProperties: toSchema(args[1] ?? 'string', index, file, definitions, referenced) }
    if (generic[1] === 'Array' || generic[1] === 'ReadonlyArray' || generic[1] === 'Set')
      return { type: 'array', items: toSchema(args[0] ?? 'string', index, file, definitions, referenced) }
    if (['Partial', 'Required', 'Readonly', 'NonNullable', 'Promise'].includes(generic[1]))
      return toSchema(args[0] ?? 'string', index, file, definitions, referenced)
    return { type: 'object', additionalProperties: { type: 'string' } }
  }
  const reference = /^([A-Z_$][\w$]*)$/i.exec(trimmed)
  if (reference) {
    const declaration = lookup(index, reference[1], file)
    if (declaration)
      return namedSchema(reference[1], declaration, index)
  }
  return { type: 'string' }
}

/** 联合成员里可合并的对象形态：内联对象直接取 properties，`$ref` 取已解析定义的 properties。 */
function objectMember(schema: Schema, definitions: Record<string, Schema>): Record<string, Schema> | undefined {
  if (schema.$ref !== undefined) {
    const name = schema.$ref.split('/').pop() ?? ''
    const target = definitions[name]
    return target?.properties
  }
  return schema.properties
}

function unionSchema(parts: string[], index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): Schema {
  const members = parts.filter(part => !NULLABLE_MEMBERS.has(part.trim())).map(part => toSchema(part, index, file, definitions, referenced))
  if (members.length === 0)
    return { type: 'string' }
  if (members.length === 1)
    return members[0]
  if (members.every(member => typeof member.enum?.[0] === 'string'))
    return { type: 'string', enum: members.map(member => member.enum?.[0] ?? '') }
  if (members.every(member => member.type === members[0].type) && members[0].properties === undefined && members[0].additionalProperties === undefined && members[0].type !== undefined)
    return { type: members[0].type }
  const shapes = members.map(member => objectMember(member, definitions))
  if (shapes.every(shape => shape !== undefined)) {
    const properties: Record<string, Schema> = {}
    const counts = new Map<string, number>()
    for (const shape of shapes) {
      for (const [name, schema] of Object.entries(shape ?? {})) {
        properties[name] ??= schema
        counts.set(name, (counts.get(name) ?? 0) + 1)
      }
    }
    if (Object.keys(properties).length === 0)
      return { type: 'object', additionalProperties: { type: 'string' } }
    const required = [...counts.entries()].filter(([, count]) => count === shapes.length).map(([name]) => name)
    return required.length > 0 ? { type: 'object', properties, required } : { type: 'object', properties }
  }
  const concrete = members.find(member => member.$ref !== undefined || member.properties !== undefined || member.type !== undefined)
  return concrete ?? { type: 'string' }
}

function fieldsSchema(fields: StatementField[], index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): Schema {
  if (fields.length === 0)
    return { type: 'object', additionalProperties: { type: 'string' } }
  const properties: Record<string, Schema> = {}
  const required: string[] = []
  for (const field of fields) {
    properties[field.name] = toSchema(field.type ?? 'string', index, file, definitions, referenced)
    if (field.required)
      required.push(field.name)
  }
  return required.length > 0 ? { type: 'object', properties, required } : { type: 'object', properties }
}

function objectSchema(type: string, index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): Schema {
  return fieldsSchema(parseFields(unwrap(type)), index, file, definitions, referenced)
}

/** 接口字段：自身字段 + `extends` 基类字段（递归解析，遇到环即停）。 */
function interfaceFields(declaration: Declaration, index: TypeIndex, seen: Set<string>): StatementField[] {
  const name = declaration.name ?? ''
  if (seen.has(name))
    return []
  seen.add(name)
  const bases = (declaration.bases ?? []).flatMap((base) => {
    const resolved = lookup(index, base, declaration.file)
    return resolved ? interfaceFields(resolved, index, seen) : []
  })
  return [...bases, ...parseFields(unwrap(`{${declaration.body}}`))]
}

/**
 * 具名类型的 schema：原样登记为 `type` 别名（schema 表达不了字面量联合/可空/泛型，
 * 逐字段近似会让 UI 端的判别联合类型失配），返回 `$ref` 供解析器引用。
 * 请求体/查询参数的顶层具名类型仍走 definition（函数签名需要 `Types.` 命名空间前缀）。
 */
function namedSchema(name: string, declaration: Declaration, index: TypeIndex): Schema {
  ensureTyping(name, declaration, index)
  return { $ref: `#/definitions/${name}` }
}

const BUILTIN_TYPE_RE = new Set(['Array', 'ReadonlyArray', 'Record', 'Map', 'Set', 'WeakMap', 'WeakSet', 'Promise', 'Partial', 'Required', 'Readonly', 'NonNullable', 'Omit', 'Pick', 'Exclude', 'Extract', 'Date', 'Error', 'RegExp', 'RequestInit', 'BodyInit', 'FormData', 'AbortSignal', 'Uint8Array'])

function declarationBody(declaration: Declaration): string {
  if (declaration.kind === 'alias')
    return declaration.body.trim()
  const object = `{${declaration.body}}`
  return declaration.bases && declaration.bases.length > 0 ? `${declaration.bases.join(' & ')} & ${object}` : object
}

/** 把具名声明连同它引用的同仓类型一起登记为 type 别名（导入别名按引用名登记，避免悬空标识符）。 */
function ensureTyping(name: string, declaration: Declaration, index: TypeIndex): void {
  if (TYPINGS.has(name))
    return
  const body = declarationBody(declaration)
  TYPINGS.set(name, body)
  for (const match of body.matchAll(/\b([A-Z_$][\w$]*)\b/g)) {
    const reference = match[1]
    if (BUILTIN_TYPE_RE.has(reference) || TYPINGS.has(reference))
      continue
    const dependency = lookup(index, reference, declaration.file)
    if (dependency)
      ensureTyping(reference, dependency, index)
  }
}

function ensureDefinition(declaration: Declaration, name: string, index: TypeIndex, definitions: Record<string, Schema>, referenced: Set<string>): void {
  if (definitions[name] !== undefined || referenced.has(name))
    return
  referenced.add(name)
  const target: Schema = {}
  definitions[name] = target
  const resolved = declaration.kind === 'interface'
    ? fieldsSchema(interfaceFields(declaration, index, new Set()), index, declaration.file, definitions, referenced)
    : toSchema(declaration.body, index, declaration.file, definitions, referenced)
  Object.assign(target, resolved)
}

function handlerParameters(source: string, operation: string, index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): HandlerParameter[] {
  const parameters: HandlerParameter[] = []
  const queryCast = /getQuery\s*\([^)]*\)\s*as\s*(\{[\s\S]*?\})\s*[;)\n]/.exec(source)
  const queryType = genericOf(source, 'getQuery')
  const queryFields = queryCast ? parseFields(queryCast[1]) : queryType?.startsWith('{') ? parseFields(queryType) : []
  for (const field of queryFields) {
    parameters.push({
      name: field.name,
      location: 'query',
      required: field.required === true,
      schema: toSchema(field.type ?? 'string', index, file, definitions, referenced),
    })
  }
  if (queryFields.length === 0 && queryType && !queryType.startsWith('{')) {
    // 具名查询契约拆成逐字段 query 参数：解析器会据此合成 `<Operation>Query`，避免 `query: X` 自引用
    const declaration = lookup(index, queryType, file)
    if (declaration && declaration.kind === 'interface') {
      for (const field of interfaceFields(declaration, index, new Set())) {
        parameters.push({
          name: field.name,
          location: 'query',
          required: field.required === true,
          schema: toSchema(field.type ?? 'string', index, declaration.file, definitions, referenced),
        })
      }
    }
    else {
      const schema = declaration ? parameterSchema(queryType, declaration, index, definitions, referenced) : { type: 'object', additionalProperties: { type: 'string' } }
      parameters.push({ name: 'query', location: 'query', required: false, schema })
    }
  }
  const bodyType = genericOf(source, 'readBody')
  if (bodyType) {
    if (bodyType.startsWith('{')) {
      const name = `${operation}Body`
      definitions[name] = objectSchema(bodyType, index, file, definitions, referenced)
      referenced.add(name)
      parameters.push({ name: 'body', location: 'body', required: true, schema: { $ref: `#/definitions/${name}` } })
      return parameters
    }
    const declaration = lookup(index, bodyType, file)
    const schema = declaration ? parameterSchema(bodyType, declaration, index, definitions, referenced) : { type: 'object', additionalProperties: { type: 'string' } }
    parameters.push({ name: 'body', location: 'body', required: true, schema })
  }
  return parameters
}

/** 请求体/查询参数的顶层具名类型：登记为 definition（函数签名需要 `Types.` 前缀）。 */
function parameterSchema(name: string, declaration: Declaration, index: TypeIndex, definitions: Record<string, Schema>, referenced: Set<string>): Schema {
  const objectLike = declaration.kind === 'interface' || declaration.body.trim().startsWith('{')
  if (!objectLike)
    return toSchema(declaration.body, index, declaration.file, definitions, referenced)
  ensureDefinition(declaration, name, index, definitions, referenced)
  return { $ref: `#/definitions/${name}` }
}

function scanArrow(input: string): string | undefined {
  const index = scan(input, 0, { open: '{[(', close: '}])', stopOn: index => input[index] === '=' && input[index + 1] === '>' }).next().value
  return index === undefined ? undefined : input.slice(0, index).trim()
}

function unwrapPromise(value: string): string {
  if (!value.startsWith('Promise') || !value.endsWith('>'))
    return value
  const open = value.indexOf('<')
  if (open < 0)
    return value
  return value.slice(open + 1, -1).trim()
}

/** 响应契约：优先取 `defineEventHandler<EventHandlerRequest, X>` 的第二个类型实参，回退到箭头返回注解。 */
function handlerResponseType(source: string): string | undefined {
  const generic = genericOf(source, 'defineEventHandler')
  if (generic !== undefined) {
    const parts = splitMembers(generic)
    if (parts.length > 1) {
      const response = parts[parts.length - 1].trim()
      if (response.length > 0)
        return unwrapPromise(response)
    }
  }
  return handlerReturnAnnotation(source)
}

function handlerReturnAnnotation(source: string): string | undefined {
  const start = RE_HANDLER.exec(source)
  if (!start)
    return undefined
  const open = source.indexOf('(', start.index)
  if (open < 0)
    return undefined
  const afterOpen = source.slice(open + 1)
  const leading = /^\s*/.exec(afterOpen)![0].length
  const withAsync = /^\s*(?:async\s*)?/.exec(afterOpen)![0].length
  const paramsOpen = source[open + 1 + withAsync] === '(' ? open + 1 + withAsync : open + 1 + leading
  if (source[paramsOpen] !== '(')
    return undefined
  const parameters = sliceBalanced(source, paramsOpen)
  if (parameters === undefined)
    return undefined
  const close = paramsOpen + parameters.length + 1
  const after = source.slice(close + 1)
  if (!/^\s*:/.test(after))
    return undefined
  const annotation = scanArrow(after.replace(/^\s*:\s*/, ''))
  return annotation ? unwrapPromise(annotation) : undefined
}

function operationResponse(source: string, index: TypeIndex, file: string, definitions: Record<string, Schema>, referenced: Set<string>): Schema | undefined {
  const annotation = handlerResponseType(source)
  if (!annotation)
    return undefined
  const candidates = splitTopLevel(annotation)
  const named = candidates.find(candidate => /^[A-Z_$][\w$]*$/i.test(candidate.trim()))
  if (named) {
    const declaration = lookup(index, named.trim(), file)
    if (declaration)
      return namedSchema(named.trim(), declaration, index)
  }
  const primitive = candidates.find(candidate => /^(?:string|number|boolean|string\[\]|number\[\]|boolean\[\])$/.test(candidate.trim()))
  if (primitive) {
    const trimmed = primitive.trim()
    return trimmed.endsWith('[]') ? { type: 'array', items: { type: trimmed.slice(0, -2) } } : { type: trimmed }
  }
  const objectLiteral = candidates.find(candidate => candidate.trim().startsWith('{'))
  if (objectLiteral)
    return objectSchema(objectLiteral, index, file, definitions, referenced)
  return undefined
}

function collectRouteFiles(routesDir: string): RouteFile[] {
  const found: RouteFile[] = []
  for (const file of walkFiles(routesDir, candidate => RE_TS_FILE.test(candidate) && !RE_SKIP_FILE.test(candidate))) {
    const parts = toPosix(relative(routesDir, file)).split('/')
    const stem = parts.pop()!.replace(/\.tsx?$/, '')
    if (stem === 'index' || stem.endsWith('.types') || stem.endsWith('.type'))
      continue
    if (!METHOD_FILES.has(stem))
      continue
    found.push({ segments: parts, method: stem, file })
  }
  return found
}

function original(configRead: ApiPipeline.ConfigRead): ApiPipeline.ConfigRead {
  const routesDir = resolve(process.cwd(), configRead.inputs.uri ?? '')
  if (!existsSync(routesDir))
    throw new Error(`genapi: 路由目录不存在 ${routesDir}，请检查 genapi.config.ts 的 input`)
  const src = resolve(routesDir, '..', '..')

  // 请求客户端同样只认 defineConfig 的 meta：ofetch 实例 + 其选项类型
  const http = configRead.config.meta?.import?.http
  if (http !== undefined) {
    configRead.graphs.scopes.main?.imports.unshift(
      { names: ['FetchOptions'], value: http, type: true },
      { names: ['ofetch'], value: http },
    )
  }

  const index = buildTypeIndex(src)
  TYPINGS.clear()
  const definitions: Record<string, Schema> = {}
  const referenced = new Set<string>()
  const paths: Record<string, Record<string, Operation>> = {}
  for (const route of collectRouteFiles(routesDir)) {
    const source = readFileSync(route.file, 'utf8')
    index.imports.set(route.file, collectImports(source, route.file))
    extractDeclarations(source, route.file, index.declarations)
    // 根级路由文件（如 routes/post.ts）对应 baseURL 本身：空路径让 transformUrlSyntax 只产出 `${baseURL}`
    const specPath = route.segments.length > 0 ? `/${route.segments.join('/')}` : ''
    const operation = operationName(route.method, route.segments)
    const parameters = handlerParameters(source, operation, index, route.file, definitions, referenced).map<OperationParameter>(parameter => parameter.location === 'body'
      ? { name: parameter.name, in: 'body', required: parameter.required, schema: parameter.schema }
      : { ...parameter.schema, name: parameter.name, in: 'query', required: parameter.required })
    const schema = operationResponse(source, index, route.file, definitions, referenced)
    const description = `${route.method.toUpperCase()} ${specPath.length > 0 ? specPath : '/'}`
    paths[specPath] ??= {}
    paths[specPath][route.method] = {
      operationId: operation,
      parameters,
      responses: { 200: schema ? { description, schema } : { description } },
    }
  }

  const reachable = new Set<string>()
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      for (const item of node)
        walk(item)
      return
    }
    if (typeof node !== 'object' || node === null)
      return
    const schema = node as Schema
    if (typeof schema.$ref === 'string') {
      const name = schema.$ref.split('/').pop()
      if (name !== undefined && !reachable.has(name)) {
        reachable.add(name)
        walk(definitions[name])
      }
      return
    }
    for (const value of Object.values(schema))
      walk(value)
  }
  for (const methods of Object.values(paths))
    walk(methods)

  const pruned: Record<string, Schema> = {}
  for (const name of reachable) {
    const definition = definitions[name]
    if (definition !== undefined)
      pruned[name] = definition
  }

  configRead.source = {
    swagger: '2.0',
    info: { title: basename(dirname(src)), version: '0.0.0' },
    paths,
    definitions: pruned,
  }
  // 字面量联合以 `type` 别名补进类型输出（schema 表达不了，否则会退化成 string）
  const typeScope = configRead.graphs.scopes.type
  if (typeScope !== undefined && TYPINGS.size > 0) {
    typeScope.typings ??= []
    for (const [name, value] of TYPINGS)
      typeScope.typings.push({ name, value, export: true })
  }
  return configRead
}

export const pluginPipeline: ApiPipeline.Pipeline = pipeline(config, original, parser, compiler, generate, dest)
