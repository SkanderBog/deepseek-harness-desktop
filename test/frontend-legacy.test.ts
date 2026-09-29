import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

/**
 * issue #761 的跨层契约：低版本 WebKit 白屏由两条独立的修复路径共同兜住，
 * 任何一条漏接都会让 macOS 10.15+ 重新白屏，而这类断点在类型检查与 lint 里都看不出来。
 *
 * ① 运行期垫片 `PROMISE_WITH_RESOLVERS_SHIM_JS` 必须在页面脚本前注入
 *    （`TypeError: Promise.withResolvers is not a function`）；
 * ② 磁盘产物降级补丁 `frontend_legacy` 必须在启动 dsh 进程前挂上
 *    （`SyntaxError: Unexpected token '{'` —— class static block 的解析期错误，
 *    垫片无济于事，只能改写文件）。
 */
const PATCH_MOD = readFileSync(
  new URL('../src-tauri/src/service/patch/mod.rs', import.meta.url),
  'utf8',
)
const LEGACY_PATCH = readFileSync(
  new URL('../src-tauri/src/service/patch/frontend_legacy.rs', import.meta.url),
  'utf8',
)
const COMPAT = readFileSync(
  new URL('../src-tauri/src/desktop/compat.rs', import.meta.url),
  'utf8',
)
const BUILDER = readFileSync(
  new URL('../src-tauri/src/desktop/builder.rs', import.meta.url),
  'utf8',
)
const PET = readFileSync(new URL('../src-tauri/src/desktop/pet.rs', import.meta.url), 'utf8')
const LAUNCH = readFileSync(
  new URL('../src-tauri/src/service/workflow/launch.rs', import.meta.url),
  'utf8',
)

describe('runtime shim for Promise.withResolvers injection', () => {
  it('declares the shim for non-Windows builds only', () => {
    // Windows 的 WebView2 是常青 Chromium（>= 119），原生就有该 API。
    const declaration = '#[cfg(not(windows))]\npub(crate) const PROMISE_WITH_RESOLVERS_SHIM_JS: &str = include_str!("compat_promise.js.inc");'
    expect(COMPAT).toContain(declaration)
  })

  it('injects the shim into every webview window builder', () => {
    const injection
      = '.initialization_script_for_all_frames(crate::desktop::compat::PROMISE_WITH_RESOLVERS_SHIM_JS)'
    // 主窗口与壳窗口各一处。
    expect(BUILDER.split(injection).length - 1).toBe(2)
    // 宠物窗口（多行参数写法）同样不能漏。
    expect(PET).toContain('crate::desktop::compat::PROMISE_WITH_RESOLVERS_SHIM_JS,')
  })

  it('injects the shim before the scripts that read it', () => {
    const shimAt = BUILDER.indexOf('PROMISE_WITH_RESOLVERS_SHIM_JS')
    const pluginBootAt = BUILDER.indexOf('PLUGIN_BOOT_RELOAD_JS')
    expect(shimAt).toBeGreaterThan(-1)
    expect(pluginBootAt).toBeGreaterThan(shimAt)
  })
})

describe('frontend legacy syntax patch', () => {
  it('registers the patch in the startup patch set', () => {
    expect(PATCH_MOD).toContain('pub(crate) mod frontend_legacy;')
    expect(PATCH_MOD).toContain('("frontend_legacy", frontend_legacy::apply_at),')
  })

  it('keeps the startup patch array length in sync with its entries', () => {
    const declared = /let patches: \[\(&str, fn\(&Path\) -> Result<\(\), String>\); (\d+)\]/.exec(
      PATCH_MOD,
    )
    expect(declared).not.toBeNull()
    const entries = PATCH_MOD.slice(PATCH_MOD.indexOf('let patches:')).match(/^\s*\("/gm)
    expect(entries).toHaveLength(Number(declared![1]))
  })

  it('hangs the patch before the dsh process starts', () => {
    expect(LAUNCH).toContain('crate::service::patch::frontend_legacy::apply(&app_handle)')
    const patchAt = LAUNCH.indexOf('frontend_legacy::apply(&app_handle)')
    const runtimeAt = LAUNCH.indexOf('prepare_active_runtime(&app_handle)')
    expect(patchAt).toBeGreaterThan(-1)
    expect(runtimeAt).toBeGreaterThan(patchAt)
  })

  it('exposes the same apply / apply_at surface as every other patch module', () => {
    expect(LEGACY_PATCH).toContain('pub fn apply_at(core_dir: &Path) -> Result<(), String>')
    expect(LEGACY_PATCH).toContain('pub fn apply(app_handle: &tauri::AppHandle) -> Result<(), String>')
  })

  it('targets the stable chunk filenames that contain class static blocks', () => {
    // 按需加载的终端 / PDF 大 chunk（共 20 处静态块）文件名稳定，直接列路径。
    expect(LEGACY_PATCH).toContain(
      '"node_modules/@deepseek-ai/dsh-client-ui-sidebar-terminal/lib/client.terminal.js"',
    )
    expect(LEGACY_PATCH).toContain(
      '"node_modules/@deepseek-ai/dsh-client-ui-sidebar-documentpreview/lib/client.pdf.js"',
    )
    expect(LEGACY_PATCH).toContain('const CHUNK_TARGETS: [&str; 2] = [')
  })

  it('resolves the hashed entry bundle instead of hard-coding one core version', () => {
    // 入口名带内容 hash，四个核心版本各不相同（index-Dy0OhsZ5.js / index-Q6zc2uHV.js /
    // index-DuF6ti6g.js / index-BKQ_L1z6.js）。写死文件名会在核心升级后失配，而
    // patch_core_file 对缺失目标只记 info 并返回 Ok —— 白屏会静默复发。
    expect(LEGACY_PATCH).toContain('fn resolve_entry_rel_path(core_dir: &Path) -> Option<String>')
    expect(LEGACY_PATCH).toContain('const WEB_FRONTEND_ASSETS_DIR')
    expect(LEGACY_PATCH).toContain('fn entry_asset_from_index_html(html: &str) -> Option<String>')
    expect(LEGACY_PATCH).toContain('if let Some(asset) = entry_asset_from_index_html(&html)')
    expect(LEGACY_PATCH).not.toContain('dist/assets/index-Dy0OhsZ5.js')
  })

  it('warns when no entry bundle can be located', () => {
    expect(LEGACY_PATCH).toContain('dsh frontend entry bundle not found under {}, skip legacy syntax patch: {}')
  })

  it('fails closed instead of writing a corrupted bundle', () => {
    // 改写前与改写后都校验整份产物的花括号收支；不平衡即放弃，宁可继续白屏。
    expect(LEGACY_PATCH).toContain('fn braces_balanced(source: &[u8]) -> bool')
    expect(LEGACY_PATCH).toContain('if !braces_balanced(source.as_bytes()) {')
    expect(LEGACY_PATCH).toContain('if !braces_balanced(text.as_bytes()) {')
    expect(LEGACY_PATCH).toContain('fn patch_fails_closed_when_braces_do_not_balance()')
  })

  it('treats a regex after `return` / `typeof` as a literal, not a division', () => {
    // `return` 的末字节是 `n`，按字节判定会把 `/}/` 当除法，正则里的 `}`
    // 随即被误判成静态块收尾，改写结果语法损坏（Unexpected token ')'）。
    expect(LEGACY_PATCH).toContain('const REGEX_KEYWORDS: [&str; 14] = [')
    expect(LEGACY_PATCH).toContain('REGEX_KEYWORDS.contains(&self.previous_word.as_str())')
    expect(LEGACY_PATCH).toContain('fn keeps_a_regex_after_return_inside_a_static_block()')
    expect(LEGACY_PATCH).toContain('fn keeps_a_regex_after_typeof_inside_a_static_block()')
  })

  it('scans JavaScript inside template `${}` interpolations', () => {
    // 模板插值里的 class expression 静态块同样要降级，否则该块仍会在旧 WebKit 上炸。
    expect(LEGACY_PATCH).toContain('templates: Vec<Option<i32>>')
    expect(LEGACY_PATCH).toContain('fn lowers_a_static_block_inside_template_interpolation()')
    expect(LEGACY_PATCH).toContain('fn ignores_static_text_inside_a_template()')
  })

  it('lowers to a static field initializer that keeps the class as `this`', () => {
    // `static { BODY }` → `static __dsh_static_block_N = (() => { BODY })();`
    // 箭头函数在类体内执行，`this` 就是类本身，且仍可访问 `#私有名`。
    expect(LEGACY_PATCH).toContain('let replacement = format!("static {FIELD_PREFIX}{counter} = (() => {{{body}}})();");')
    // 结尾分号不可省：压缩产物里静态块紧邻下一个类成员（`}constructor(`）。
    expect(LEGACY_PATCH).toContain('assert!(patched.ends_with(')
  })

  it('writes an idempotency marker into the patched bundle', () => {
    expect(LEGACY_PATCH).toContain('issue #761) */";')
    expect(LEGACY_PATCH).toContain('if source.contains(PATCH_MARKER) {')
  })

  it('only rewrites `static` in code context, never inside strings or comments', () => {
    // client.pdf.js 内嵌了 pdf worker 的源码字符串，其中含字面量 `static{`。
    expect(LEGACY_PATCH).toContain('fn next_code(&mut self) -> Option<(usize, u8)>')
    expect(LEGACY_PATCH).toContain('fn scan_regex(source: &[u8], start: usize) -> Option<usize>')
    expect(LEGACY_PATCH).toContain('fn ignores_static_outside_code_context()')
  })

  it('lowers nested static blocks in inner-first passes', () => {
    // 两个大 chunk 里存在类中类，外层块体内还嵌着内层块，必须多轮处理。
    expect(LEGACY_PATCH).toContain('let innermost: Vec<(usize, usize, usize)> = blocks')
    expect(LEGACY_PATCH).toContain('for (start, brace, end) in innermost.into_iter().rev()')
  })
})
