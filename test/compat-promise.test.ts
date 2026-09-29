import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { describe, expect, it } from 'vitest'

/**
 * issue #761：macOS 14.x 及更早随附的系统 WebKit 没有 `Promise.withResolvers`
 *（Safari 17.4 / WebKit 1691 才引入）。dsh 宿主在 serve `index.html` 时于末尾内联
 * 启动握手脚本 `(globalThis.__DSH_BOOT_READY__ ??= Promise.withResolvers()).resolve()`，
 * 旧 WebKit 上它在解析当期即抛
 * `TypeError: Promise.withResolvers is not a function. (In 'Promise.withResolvers()',
 * 'Promise.withResolvers' is undefined)`，握手失败、页面白屏。
 *
 * 壳层垫片 `src-tauri/src/desktop/compat_promise.js.inc` 在页面脚本前注入。这里在 VM
 * 里删掉原生 `Promise.withResolvers` 模拟旧 WebKit，并把垫片行为与 Node（V8，原生实现）
 * 做差分对照。
 */
const shim = readFileSync(
  new URL('../src-tauri/src/desktop/compat_promise.js.inc', import.meta.url),
  'utf8',
)

/** 旧 WebKit 现场：`Promise` 上不存在 `withResolvers`。 */
const OLD_WEBKIT_SETUP = `
delete Promise.withResolvers
if (typeof Promise.withResolvers === 'function') {
  Object.defineProperty(Promise, 'withResolvers', { value: undefined, configurable: true })
}
`

/** dsh 宿主 serve index.html 时注入的启动握手（垫片生效前它就是 issue #761 的报错点）。 */
const BOOT_HANDSHAKE
  = '(globalThis.__DSH_BOOT_READY__ ??= Promise.withResolvers()).resolve()'

/**
 * 对照探针：返回 JSON 字符串，跨 realm 只比较字符串，避免原型差异干扰。
 */
const PROBE = `(async function () {
  async function settle(promise) {
    try { return { value: await promise } }
    catch (error) { return { rejected: error.message } }
  }
  var out = {}
  var descriptor = Object.getOwnPropertyDescriptor(Promise, 'withResolvers')
  out.descriptor = descriptor ? {
    configurable: descriptor.configurable,
    writable: descriptor.writable,
    enumerable: descriptor.enumerable,
    callable: typeof descriptor.value === 'function',
  } : null
  out.enumerableKey = Object.keys(Promise).indexOf('withResolvers') !== -1
  var first = Promise.withResolvers()
  out.shape = [first.promise instanceof Promise, typeof first.resolve, typeof first.reject]
  first.resolve('done')
  out.resolved = await settle(first.promise)
  var second = Promise.withResolvers()
  second.reject(new Error('nope'))
  out.rejected = await settle(second.promise)
  var third = Promise.withResolvers()
  out.pending = await Promise.race([
    third.promise.then(function () { return 'settled' }),
    Promise.resolve('pending'),
  ])
  out.distinct = Promise.withResolvers().promise !== Promise.withResolvers().promise
  return JSON.stringify(out)
})()`

/** 同步探针：只验证返回形态，用于「让位与幂等」判定。 */
const SYNC_PROBE = `JSON.stringify([
  typeof Promise.withResolvers,
  typeof Promise.withResolvers().resolve,
])`

function createSandbox(): Record<string, unknown> {
  const sandbox: Record<string, unknown> = {}
  // 浏览器语义：window 就是全局对象。
  sandbox.window = sandbox
  return sandbox
}

function runInSandbox(code: string, sandbox: Record<string, unknown>): unknown {
  return runInNewContext(code, sandbox)
}

describe('runtime shim for Promise.withResolvers (issue #761)', () => {
  it('reproduces the boot handshake failure when the API is missing', () => {
    const sandbox = createSandbox()
    // 匹配报错文本而非 TypeError 构造器：VM 里抛出的 TypeError 属于另一个 realm，
    // `instanceof TypeError` 在宿主 realm 判定为假。
    expect(() => runInSandbox(`${OLD_WEBKIT_SETUP}\n;\n${BOOT_HANDSHAKE}`, sandbox))
      .toThrow(/Promise\.withResolvers is not a function/)
  })

  it('lets the boot handshake resolve once the shim is injected', () => {
    const sandbox = createSandbox()
    expect(() =>
      runInSandbox(`${OLD_WEBKIT_SETUP}\n;\n${shim}\n;\n${BOOT_HANDSHAKE}`, sandbox),
    ).not.toThrow()

    expect(typeof sandbox.__DSH_BOOT_READY__).toBe('object')
    expect((sandbox.window as Record<string, unknown>).__dsh_promise_with_resolvers__).toBe(true)
  })

  it('matches the host engine native Promise.withResolvers value for value', async () => {
    const oldWebKit = createSandbox()
    const polyfilled = JSON.parse(
      await runInSandbox(`${OLD_WEBKIT_SETUP}\n;\n${shim}\n;\n${PROBE}`, oldWebKit) as string,
    ) as Record<string, unknown>

    // 同一探针跑在原生实现上（垫片检测到 API 已存在会整体让位）作为基准。
    const modern = createSandbox()
    const native = JSON.parse(
      await runInSandbox(`${shim}\n;\n${PROBE}`, modern) as string,
    ) as Record<string, unknown>

    expect(polyfilled).toEqual(native)
  })

  it('yields to an existing Promise.withResolvers and stays idempotent across injections', () => {
    const modern = createSandbox()
    runInSandbox(shim, modern)
    // 原生已有该 API：不改写 Promise、不落幂等标记。
    expect(JSON.parse(runInSandbox(SYNC_PROBE, modern) as string)).toEqual(['function', 'function'])
    expect((modern.window as Record<string, unknown>).__dsh_promise_with_resolvers__).toBeUndefined()

    // 重复注入必须在 VM 内部比较：contextified sandbox 不会把内建对象映射回宿主对象，
    // 宿主的 `sandbox.Promise` 永远是 undefined。
    const oldWebKit = createSandbox()
    const identity = runInSandbox(
      [
        OLD_WEBKIT_SETUP,
        shim,
        'globalThis.__dsh_first_with_resolvers__ = Promise.withResolvers;',
        shim,
        'JSON.stringify([',
        '  typeof Promise.withResolvers,',
        '  Promise.withResolvers === globalThis.__dsh_first_with_resolvers__,',
        '])',
      ].join('\n'),
      oldWebKit,
    )
    expect(JSON.parse(identity as string)).toEqual(['function', true])
  })
})
