//! 低版本 WebKit 产物降级补丁：把 class static block 改写成等价的静态字段初始化器。
//!
//! 背景：`dsh-web-frontend` 的浏览器产物（以及按需加载的终端 / PDF 大 chunk）使用了
//! class static block 语法（`static { ... }`，ES2022）。该语法要求 Safari 16.4 /
//! WebKit 1660，而桌面端声明支持的 macOS 10.15+ 系统 WebKit 远低于此，模块在解析期
//! 就抛 `SyntaxError: Unexpected token '{'`（issue #761 日志里的
//! `assets/index-Dy0OhsZ5.js:9:0` 正对应产物中第一处静态块所在行），前端入口整体
//! 无法加载、页面白屏。语法错误无法用运行期垫片弥补——必须在文件被读取前改写掉。
//!
//! 改写形态：`static { BODY }` → `static __dsh_static_block_N = (() => { BODY })();`
//!
//! - 静态字段初始化器里的箭头函数以类自身为 `this`，与静态块的 `this` 语义一致；
//! - 箭头函数定义在类体内，仍可访问 `#私有名`（两个大 chunk 的静态块都用了）；
//! - 目标产物的静态块均不含 `super` / `arguments` / `new.target`；
//! - 静态字段本身只需 Safari 14.1，落在产物既有语法基线之内；
//! - 结尾分号不可省：压缩产物里静态块紧邻下一个类成员（`}constructor(`），
//!   没有分号时字段初始化器与后续成员之间无法靠 ASI 分隔。
//!
//! `static` 只在**代码上下文**识别：`client.pdf.js` 内嵌了 pdf worker 的源码字符串，
//! 其中含字面量 `static{`，改写它会破坏字符串内容。故这里带一个精简的 JS 词法状态机
//! （字符串 / 模板串及其 `${}` 插值 / 行注释 / 块注释 / 正则字面量），只对代码上下文
//! 中的静态块动刀。
//!
//! 目标选择：入口产物名带内容 hash，各核心版本互不相同（`index-Dy0OhsZ5.js` ↔
//! `index-Q6zc2uHV.js` ↔ …），写死文件名会在核心升级后静默失配、白屏复发，故入口按
//! `dist/index.html` 的引用解析、再退化到枚举 `dist/assets/index-*.js`；两个按需加载
//! 的大 chunk 文件名稳定，直接列路径。读写与幂等判定统一交给
//! [`crate::utils::patch_core_file`]，本模块只提供纯函数式补丁判定 [`patch_source`]。
//!
//! 改写前后都校验整份产物的花括号收支（`{}` 必须配对且中途不为负），任一不成立即放弃
//! 本次补丁（[`PatchOutcome::AnchorMissing`]）——宁可继续白屏，也不写出语法损坏的产物。
//!
//! 幂等：改写时在文件首行写入 [`PATCH_MARKER`]，后续启动据此跳过；文件里已没有静态块
//! （上游将来改了构建目标）同样跳过，本模块自动退休。
//!
//! 挂点：`service::workflow::launch` 启动 dsh 进程前（最佳努力，失败只告警）。

use std::path::Path;

use crate::utils::{active_core_install_dir, patch_core_file, PatchOutcome};

/// 首行标记：既用于跳过重复补丁，也让产物来源一目了然。
const PATCH_MARKER: &str =
    "/* dsh-tauri: legacy WebKit class static block lowering (issue #761) */";

/// 改写后静态字段名的前缀，按静态块出现顺序追加序号保证唯一。
const FIELD_PREFIX: &str = "__dsh_static_block_";

/// `dsh-web-frontend` 产物目录（相对活动核心安装目录）。
const WEB_FRONTEND_DIST_DIR: &str = "node_modules/@deepseek-ai/dsh-web-frontend/dist";

/// 同一产物的 `assets` 子目录：入口文件名带 hash，只能枚举。
const WEB_FRONTEND_ASSETS_DIR: &str = "node_modules/@deepseek-ai/dsh-web-frontend/dist/assets";

/// 按需加载的两个大 chunk：文件名稳定，直接列路径。
///
/// 这两个 chunk 不在包 `exports` 映射内，但由 `client.js` 里的
/// `require.async("./client.terminal.js")` / `require.async("./client.pdf.js")`
/// 在用户打开终端、预览 PDF 时加载，同样要经过旧 WebKit 的解析器。
///
/// 早期核心（0.1.5-rc.x）不含这两个文件，[`patch_core_file`] 对缺失目标静默跳过属正常路径。
const CHUNK_TARGETS: [&str; 2] = [
    "node_modules/@deepseek-ai/dsh-client-ui-sidebar-terminal/lib/client.terminal.js",
    "node_modules/@deepseek-ai/dsh-client-ui-sidebar-documentpreview/lib/client.pdf.js",
];

/// 允许紧跟在后面出现正则字面量的关键字。
///
/// 判定 `/` 是正则还是除号不能只看前一个字节：`return` / `typeof` 的末字节是 `n`，
/// 按字节判定会把它们后面的正则当成除法，于是正则体内的 `}` 被误判成静态块的收尾，
/// 改写结果直接语法损坏（`Unexpected token ')'`）。
const REGEX_KEYWORDS: [&str; 14] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "do",
    "else",
    "case",
    "yield",
    "await",
];

/// 幂等补丁逻辑的纯函数部分（便于单测，不触碰文件系统）。
fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if lex_static_blocks(source.as_bytes()).is_empty() {
        return PatchOutcome::AlreadyPatched;
    }
    // 拿不准就不动：扫描结果不可信时改写只会把产物写坏。
    if !braces_balanced(source.as_bytes()) {
        return PatchOutcome::AnchorMissing;
    }

    let mut text = source.to_string();
    let mut counter = 0usize;
    loop {
        let marks = lex_static_blocks(text.as_bytes());
        if marks.is_empty() {
            break;
        }
        let mut blocks = Vec::with_capacity(marks.len());
        for (start, brace) in marks {
            let Some(end) = match_block_brace(text.as_bytes(), brace) else {
                return PatchOutcome::AnchorMissing;
            };
            blocks.push((start, brace, end));
        }
        // 只降级最内层的静态块：体内还嵌着静态块的外层留到下一轮，届时内层已完成
        // 改写，外层取到的是更新后的文本与偏移。
        let innermost: Vec<(usize, usize, usize)> = blocks
            .iter()
            .copied()
            .filter(|(start, _, end)| {
                !blocks
                    .iter()
                    .any(|(other, _, _)| other > start && other < end)
            })
            .collect();
        if innermost.is_empty() {
            return PatchOutcome::AnchorMissing;
        }
        // 逆序改写：靠后的块先替换，靠前块的偏移保持有效。
        for (start, brace, end) in innermost.into_iter().rev() {
            let body = &text[brace + 1..end];
            let replacement = format!("static {FIELD_PREFIX}{counter} = (() => {{{body}}})();");
            counter += 1;
            text.replace_range(start..end + 1, &replacement);
        }
    }

    if !braces_balanced(text.as_bytes()) {
        return PatchOutcome::AnchorMissing;
    }

    let mut patched = String::with_capacity(text.len() + PATCH_MARKER.len() + 1);
    patched.push_str(PATCH_MARKER);
    patched.push('\n');
    patched.push_str(&text);
    PatchOutcome::Patched(patched)
}

/// 对活动核心的目标产物逐一应用补丁（幂等）。
///
/// 单文件失败不阻断其余（与 [`crate::service::patch::apply_all_at`] 的最佳努力语义一致），
/// 失败信息汇总返回，只有真实读写失败才带 `DSH_PATCH_` 前缀。
pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    match resolve_entry_rel_path(core_dir) {
        Some(rel_path) => {
            if let Err(error) = patch_core_file(core_dir, &rel_path, patch_source) {
                failures.push(error);
            }
        }
        None => log::warn!(
            "dsh frontend entry bundle not found under {}, skip legacy syntax patch: {}",
            core_dir.display(),
            WEB_FRONTEND_ASSETS_DIR
        ),
    }
    for rel_path in CHUNK_TARGETS {
        if let Err(error) = patch_core_file(core_dir, rel_path, patch_source) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// [`apply_at`] 的活动核心版本（拿 `AppHandle` 定位核心安装目录）。
pub fn apply(app_handle: &tauri::AppHandle) -> Result<(), String> {
    apply_at(&active_core_install_dir(app_handle))
}

/// 从 `dist/index.html` 里取出模块入口产物名（相对 `dist`），例如 `assets/index-Dy0OhsZ5.js`。
///
/// 同一份 html 里还有 `assets/index-*.css`，故只在以 `.js` 结尾的引用上命中。
fn entry_asset_from_index_html(html: &str) -> Option<String> {
    const NEEDLE: &str = "assets/index-";
    let mut from = 0usize;
    while let Some(offset) = html[from..].find(NEEDLE) {
        let start = from + offset;
        let rest = &html[start..];
        let end = rest
            .find(|c: char| {
                c == '"' || c == '\'' || c == '?' || c == '#' || c.is_whitespace()
            })
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if name.ends_with(".js") {
            return Some(name.to_string());
        }
        from = start + NEEDLE.len();
    }
    None
}

/// 定位入口产物相对活动核心安装目录的路径。
///
/// 优先信 `index.html`（那才是浏览器真正加载的文件），引用缺失或文件不存在时退化为
/// 枚举 `dist/assets` 下的 `index-*.js`；两者都拿不到返回 `None`，由调用方告警。
fn resolve_entry_rel_path(core_dir: &Path) -> Option<String> {
    let dist_dir = core_dir.join(WEB_FRONTEND_DIST_DIR);
    if let Ok(html) = std::fs::read_to_string(dist_dir.join("index.html")) {
        if let Some(asset) = entry_asset_from_index_html(&html) {
            if dist_dir.join(&asset).is_file() {
                return Some(format!("{WEB_FRONTEND_DIST_DIR}/{asset}"));
            }
        }
    }
    let mut candidates: Vec<String> = std::fs::read_dir(core_dir.join(WEB_FRONTEND_ASSETS_DIR))
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("index-") && name.ends_with(".js"))
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|name| format!("{WEB_FRONTEND_ASSETS_DIR}/{name}"))
}

/// 词法状态机：把 JS 源码切成「代码字符」与「被跳过的字面量 / 注释」。
///
/// 只跟踪判定正则字面量与类静态块所必需的状态，不做完整解析——目标产物的语法
/// 完全落在标准 JS 内，这套精简扫描在上游三份产物上与真实解析结果一致。
struct Scanner<'a> {
    source: &'a [u8],
    index: usize,
    string_quote: Option<u8>,
    /// 模板串栈：空 = 普通代码态；`None` = 模板文本；`Some(depth)` = `${...}` 代码态。
    templates: Vec<Option<i32>>,
    in_line_comment: bool,
    in_block_comment: bool,
    /// 最后一个有意义的代码字节（空白不更新），正则 / 除号的字节级判据。
    previous: Option<u8>,
    /// 紧邻 `previous` 的标识符 / 关键字，用于识别 `return /re/` 一类场景。
    previous_word: String,
    /// `previous` 与当前位置之间是否存在空白，决定标识符是否续接。
    previous_space: bool,
}

impl<'a> Scanner<'a> {
    fn new(source: &'a [u8], index: usize) -> Self {
        Self {
            source,
            index,
            string_quote: None,
            templates: Vec::new(),
            in_line_comment: false,
            in_block_comment: false,
            previous: None,
            previous_word: String::new(),
            previous_space: false,
        }
    }

    /// 返回下一个代码字符 `(偏移, 字节)`；字面量与注释整体跳过，扫完返回 `None`。
    fn next_code(&mut self) -> Option<(usize, u8)> {
        let source = self.source;
        let length = source.len();
        while self.index < length {
            let index = self.index;
            let byte = source[index];
            let next = source.get(index + 1).copied();
            if self.in_line_comment {
                self.index += 1;
                if byte == b'\n' {
                    self.in_line_comment = false;
                }
                continue;
            }
            if self.in_block_comment {
                if byte == b'*' && next == Some(b'/') {
                    self.in_block_comment = false;
                    self.index += 2;
                } else {
                    self.index += 1;
                }
                continue;
            }
            if let Some(quote) = self.string_quote {
                if byte == b'\\' {
                    self.index += 2;
                    continue;
                }
                if byte == quote {
                    self.string_quote = None;
                    self.index += 1;
                    self.push(byte);
                    continue;
                }
                self.index += 1;
                continue;
            }
            // 模板：文本态只看转义、收尾反引号与 `${`；`${}` 内按普通代码继续往下走。
            if let Some(top) = self.templates.last().copied() {
                match top {
                    None => {
                        if byte == b'\\' {
                            self.index += 2;
                            continue;
                        }
                        if byte == b'`' {
                            self.templates.pop();
                            self.index += 1;
                            self.push(byte);
                            continue;
                        }
                        if byte == b'$' && next == Some(b'{') {
                            if let Some(slot) = self.templates.last_mut() {
                                *slot = Some(0);
                            }
                            // `${` 开启一个新的表达式，正则判定回到「表达式起点」。
                            self.push(b'{');
                            self.index += 2;
                            continue;
                        }
                        self.index += 1;
                        continue;
                    }
                    Some(depth) => {
                        if byte == b'{' {
                            if let Some(slot) = self.templates.last_mut() {
                                *slot = Some(depth + 1);
                            }
                            self.index += 1;
                            return Some((index, byte));
                        }
                        if byte == b'}' {
                            if depth == 0 {
                                // 插值收尾：回到模板文本态，这个 `}` 不返回给调用方。
                                if let Some(slot) = self.templates.last_mut() {
                                    *slot = None;
                                }
                                self.index += 1;
                                continue;
                            }
                            if let Some(slot) = self.templates.last_mut() {
                                *slot = Some(depth - 1);
                            }
                            self.index += 1;
                            return Some((index, byte));
                        }
                    }
                }
            }
            if byte == b'/' && next == Some(b'/') {
                self.in_line_comment = true;
                self.index += 2;
                continue;
            }
            if byte == b'/' && next == Some(b'*') {
                self.in_block_comment = true;
                self.index += 2;
                continue;
            }
            if byte == b'/' && self.regex_expected() {
                if let Some(after) = scan_regex(source, index) {
                    self.index = after;
                    self.push(b'/');
                    continue;
                }
            }
            if byte == b'\'' || byte == b'"' {
                self.string_quote = Some(byte);
                self.index += 1;
                continue;
            }
            if byte == b'`' {
                self.templates.push(None);
                self.index += 1;
                continue;
            }
            self.index += 1;
            return Some((index, byte));
        }
        None
    }

    fn push(&mut self, byte: u8) {
        if byte.is_ascii_whitespace() {
            self.previous_space = true;
            return;
        }
        if is_word_byte(byte) {
            if self.previous_space || !self.previous.is_some_and(is_word_byte) {
                self.previous_word.clear();
            }
            self.previous_word.push(byte as char);
        } else {
            self.previous_word.clear();
        }
        self.previous_space = false;
        self.previous = Some(byte);
    }

    /// `/` 是正则字面量开头还是除号：行首 / 运算符之后，或前一 token 是关键字时为正则。
    fn regex_expected(&self) -> bool {
        match self.previous {
            None => true,
            Some(byte) if is_word_byte(byte) => {
                REGEX_KEYWORDS.contains(&self.previous_word.as_str())
            }
            Some(byte) => matches!(
                byte,
                b'=' | b'('
                    | b','
                    | b':'
                    | b'['
                    | b'!'
                    | b'&'
                    | b'|'
                    | b'?'
                    | b'{'
                    | b'}'
                    | b';'
                    | b'+'
                    | b'-'
                    | b'*'
                    | b'%'
                    | b'^'
                    | b'~'
                    | b'<'
                    | b'>'
            ),
        }
    }
}

/// 从 `start` 处的 `/` 起扫描正则字面量，返回结束（含 flags）后的偏移；未闭合返回 `None`。
fn scan_regex(source: &[u8], start: usize) -> Option<usize> {
    let length = source.len();
    let mut index = start + 1;
    let mut in_class = false;
    let mut closed = false;
    while index < length {
        let byte = source[index];
        if byte == b'\\' {
            index += 2;
            continue;
        }
        if byte == b'\n' {
            break;
        }
        if byte == b'[' {
            in_class = true;
        } else if byte == b']' {
            in_class = false;
        } else if byte == b'/' && !in_class {
            closed = true;
            break;
        }
        index += 1;
    }
    if !closed {
        return None;
    }
    let mut after = index + 1;
    while after < length && source[after].is_ascii_alphabetic() {
        after += 1;
    }
    Some(after)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// 整份产物的花括号收支：代码上下文里的 `{}` 必须配对，中途不得为负。
fn braces_balanced(source: &[u8]) -> bool {
    let mut scanner = Scanner::new(source, 0);
    let mut depth = 0i32;
    while let Some((_, byte)) = scanner.next_code() {
        if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
        scanner.push(byte);
    }
    depth == 0
}

/// 收集代码上下文里类静态块的位置，返回 `(static 偏移, 左花括号偏移)`，按出现顺序。
fn lex_static_blocks(source: &[u8]) -> Vec<(usize, usize)> {
    let mut scanner = Scanner::new(source, 0);
    let mut marks = Vec::new();
    while let Some((index, byte)) = scanner.next_code() {
        if byte == b's' && source[index..].starts_with(b"static") {
            let previous_ok = index == 0 || !is_word_byte(source[index - 1]);
            let next_ok = source.len() <= index + 6 || !is_word_byte(source[index + 6]);
            if previous_ok && next_ok {
                let mut brace = index + 6;
                while brace < source.len() && source[brace].is_ascii_whitespace() {
                    brace += 1;
                }
                if source.get(brace) == Some(&b'{') {
                    marks.push((index, brace));
                }
            }
        }
        scanner.push(byte);
    }
    marks
}

/// 从 `open` 处的 `{` 起配对其右花括号，返回其偏移；未配对返回 `None`。
fn match_block_brace(source: &[u8], open: usize) -> Option<usize> {
    let mut scanner = Scanner::new(source, open);
    let mut depth = 0i32;
    let mut started = false;
    while let Some((index, byte)) = scanner.next_code() {
        if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if started && depth == 0 {
                return Some(index);
            }
        }
        started = true;
        scanner.push(byte);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_static_block_into_field_initializer() {
        let source = "class A {\n  static {\n    this.value = 1;\n  }\n  method() {}\n}\n";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.starts_with(PATCH_MARKER));
                assert!(patched.contains(
                    "static __dsh_static_block_0 = (() => {\n    this.value = 1;\n  })();"
                ));
                assert!(!patched.contains("static {"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn keeps_body_verbatim_including_nested_braces_and_strings() {
        let source = "class A{static{const o={a:1};const s=\"}\";this.y=o.a;}}";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(
                    "static __dsh_static_block_0 = (() => {const o={a:1};const s=\"}\";this.y=o.a;})();"
                ));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn terminates_field_initializer_before_next_member() {
        // 压缩产物里静态块紧邻下一个类成员，缺分号时 ASI 无法分隔，
        // 解析器会直接报 `Unexpected identifier 'constructor'`。
        let source = "class A{static{this.V=1}constructor(){}}";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.ends_with(
                    "class A{static __dsh_static_block_0 = (() => {this.V=1})();constructor(){}}"
                ));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn numbers_multiple_blocks_uniquely() {
        let source = "class A{static{this.a=1}}class B{static{this.b=2}}";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains("__dsh_static_block_0"));
                assert!(patched.contains("__dsh_static_block_1"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn ignores_static_outside_code_context() {
        // `client.pdf.js` 内嵌了 pdf worker 源码字符串，其中含字面量 `static{`；
        // 字符串 / 注释 / 正则里的 static 一律不得改写。
        let source = concat!(
            "const text = 'static {';\n",
            "const pattern = /static \\{/;\n",
            "// static {\n",
            "/* static { */\n",
            "class A { static { this.x = 1; } }\n"
        );
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert_eq!(patched.matches(FIELD_PREFIX).count(), 1);
                assert!(patched.contains("const text = 'static {';"));
                assert!(patched.contains("const pattern = /static \\{/;"));
                assert!(patched.contains("// static {"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn keeps_a_regex_after_return_inside_a_static_block() {
        // `return` 的末字节是 `n`，只看字节会把 `/}/` 当成除法，正则体内的 `}`
        // 随即被误判为静态块收尾，块体被截断、产物语法损坏。
        let source = "class A{static{function f(){return /}/.test(\"\")}this.x=1}}";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(r#"return /}/.test("")"#));
                assert!(patched.ends_with("this.x=1})();}"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn keeps_a_regex_after_typeof_inside_a_static_block() {
        // `typeof /}/` —— 修复前会输出 `... typeof /})();/;...`，正则跨过收尾而破损。
        let source = "class A{static{const r=typeof /}/;this.x=1}}";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains("const r=typeof /}/;this.x=1})();"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn lowers_a_static_block_inside_template_interpolation() {
        // 模板 `${}` 内是代码上下文，其中的 class expression 静态块同样要降级。
        let source = "const a = `x${(class B{static{this.y=2}}).y}`;\nclass A{static{this.x=1}}\n";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert_eq!(patched.matches(FIELD_PREFIX).count(), 2);
                assert!(!patched.contains("static{"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn ignores_static_text_inside_a_template() {
        let source = "const t = `static { not code }`;\nclass A{static{this.x=1}}\n";
        match patch_source(source) {
            PatchOutcome::Patched(patched) => {
                assert_eq!(patched.matches(FIELD_PREFIX).count(), 1);
                assert!(patched.contains("`static { not code }`"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn skips_source_without_static_block() {
        let source = "class A { static value = 1; }\n";
        assert_eq!(patch_source(source), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn patch_is_idempotent() {
        let source = "class A { static { this.x = 1; } }\n";
        let PatchOutcome::Patched(patched) = patch_source(source) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn patch_skips_when_brace_unmatched() {
        let source = "class A { static { this.x = 1;\n";
        assert_eq!(patch_source(source), PatchOutcome::AnchorMissing);
    }

    #[test]
    fn patch_fails_closed_when_braces_do_not_balance() {
        // 静态块本身配对，但整份产物多了一个 `}` —— 扫描结果不可信，宁可不改。
        let source = "class A{static{this.x=1}}}";
        assert_eq!(patch_source(source), PatchOutcome::AnchorMissing);
    }

    #[test]
    fn resolves_entry_bundle_from_index_html_reference() {
        let html = concat!(
            "<script type=\"module\" crossorigin src=\"./assets/index-Dy0OhsZ5.js\"></script>",
            "<link rel=\"stylesheet\" crossorigin href=\"./assets/index-Cq6ljTv2.css\">"
        );
        assert_eq!(
            entry_asset_from_index_html(html).as_deref(),
            Some("assets/index-Dy0OhsZ5.js")
        );
    }

    #[test]
    fn resolves_entry_bundle_when_stylesheet_precedes_the_module() {
        // 引用顺序可能变，只认以 `.js` 结尾的那条。
        let html = concat!(
            "<link rel=\"stylesheet\" href=\"./assets/index-Cq6ljTv2.css\">",
            "<script src=\"./assets/index-Q6zc2uHV.js\"></script>"
        );
        assert_eq!(
            entry_asset_from_index_html(html).as_deref(),
            Some("assets/index-Q6zc2uHV.js")
        );
    }

    #[test]
    fn entry_resolution_falls_back_to_none_without_a_module_reference() {
        let html = "<link rel=\"stylesheet\" href=\"./assets/index-Cq6ljTv2.css\">";
        assert_eq!(entry_asset_from_index_html(html), None);
    }

    #[test]
    fn chunk_targets_are_stable_filenames_outside_the_hashed_entry() {
        assert_eq!(CHUNK_TARGETS.len(), 2);
        assert!(CHUNK_TARGETS
            .iter()
            .all(|path| path.starts_with("node_modules/@deepseek-ai/")));
        // 入口名带内容 hash，只能解析，不能出现在写死的目标里。
        assert!(CHUNK_TARGETS.iter().all(|path| !path.contains("dist/assets/index-")));
    }
}
