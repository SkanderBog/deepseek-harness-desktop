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
//! - 三处目标产物的静态块均不含 `super` / `arguments` / `new.target`；
//! - 静态字段本身只需 Safari 14.1，落在产物既有语法基线之内；
//! - 结尾分号不可省：压缩产物里静态块紧邻下一个类成员（`}constructor(`），
//!   没有分号时字段初始化器与后续成员之间无法靠 ASI 分隔。
//!
//! `static` 只在**代码上下文**识别：`client.pdf.js` 内嵌了 pdf worker 的源码字符串，
//! 其中含字面量 `static{`，改写它会破坏字符串内容。故这里带一个精简的 JS 词法状态机
//! （字符串 / 模板串 / 行注释 / 块注释 / 正则字面量），只对代码上下文中的静态块动刀。
//!
//! 目标选择：**活动核心**里三个体积与命中数都已知的产物文件，读写与幂等判定统一交给
//! [`crate::utils::patch_dsh`]；本模块只提供纯函数式补丁判定 [`patch_source`]。
//!
//! 幂等：改写时在文件首行写入 [`PATCH_MARKER`]，后续启动据此跳过；文件里已没有静态块
//! （上游将来改了构建目标）同样跳过，本模块自动退休。
//!
//! 挂点：`service::workflow::launch` 启动 dsh 进程前（最佳努力，失败只告警）。

use std::path::Path;

use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};

/// 首行标记：既用于跳过重复补丁，也让产物来源一目了然。
const PATCH_MARKER: &str =
    "/* dsh-tauri: legacy WebKit class static block lowering (issue #761) */";

/// 改写后静态字段名的前缀，按静态块出现顺序追加序号保证唯一。
const FIELD_PREFIX: &str = "__dsh_static_block_";

/// 需要降级的三个相对活动核心安装目录的产物路径。
///
/// 前两个大 chunk 不在包 `exports` 映射内，但由 `client.js` 里的
/// `require.async("./client.terminal.js")` / `require.async("./client.pdf.js")`
/// 在用户打开终端、预览 PDF 时按需加载，同样要经过旧 WebKit 的解析器。
const TARGETS: [&str; 3] = [
    "node_modules/@deepseek-ai/dsh-web-frontend/dist/assets/index-Dy0OhsZ5.js",
    "node_modules/@deepseek-ai/dsh-client-ui-sidebar-terminal/lib/client.terminal.js",
    "node_modules/@deepseek-ai/dsh-client-ui-sidebar-documentpreview/lib/client.pdf.js",
];

/// 幂等补丁逻辑的纯函数部分（便于单测，不触碰文件系统）。
fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if lex_static_blocks(source.as_bytes()).is_empty() {
        return PatchOutcome::AlreadyPatched;
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

    let mut patched = String::with_capacity(text.len() + PATCH_MARKER.len() + 1);
    patched.push_str(PATCH_MARKER);
    patched.push('\n');
    patched.push_str(&text);
    PatchOutcome::Patched(patched)
}

/// 对活动核心的三个目标产物逐一应用补丁（幂等）。
///
/// 单文件失败不阻断其余（与 [`crate::service::patch::apply_all_at`] 的最佳努力语义一致），
/// 失败信息汇总返回，只有真实读写失败才带 `DSH_PATCH_` 前缀。
pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    for rel_path in TARGETS {
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
    let mut failures = Vec::new();
    for rel_path in TARGETS {
        if let Err(error) = patch_dsh(app_handle, rel_path, patch_source) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// 词法状态机：把 JS 源码切成「代码字符」与「被跳过的字面量 / 注释」。
///
/// 只跟踪判定正则字面量与类静态块所必需的状态，不做完整解析——目标产物的语法
/// 完全落在标准 JS 内，这套精简扫描在上游三份产物上与真实解析结果一致。
struct Scanner<'a> {
    source: &'a [u8],
    index: usize,
    string_quote: Option<u8>,
    in_template: bool,
    in_line_comment: bool,
    in_block_comment: bool,
    previous: Vec<u8>,
}

impl<'a> Scanner<'a> {
    fn new(source: &'a [u8], index: usize) -> Self {
        Self {
            source,
            index,
            string_quote: None,
            in_template: false,
            in_line_comment: false,
            in_block_comment: false,
            previous: Vec::new(),
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
            if self.string_quote.is_some() || self.in_template {
                if byte == b'\\' {
                    self.index += 2;
                    continue;
                }
                if self.string_quote == Some(byte) {
                    self.string_quote = None;
                    self.index += 1;
                    self.push(byte);
                    continue;
                }
                if self.in_template && byte == b'`' {
                    self.in_template = false;
                    self.index += 1;
                    self.push(byte);
                    continue;
                }
                self.index += 1;
                continue;
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
                self.in_template = true;
                self.index += 1;
                continue;
            }
            self.index += 1;
            return Some((index, byte));
        }
        None
    }

    fn push(&mut self, byte: u8) {
        if !byte.is_ascii_whitespace() {
            self.previous.push(byte);
        }
    }

    /// 前一个有效字符决定 `/` 是正则字面量开头还是除号（行首或运算符之后即正则）。
    fn regex_expected(&self) -> bool {
        match self.previous.last().copied() {
            None => true,
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

/// 收集代码上下文里类静态块的位置，返回 `(static 偏移, 左花括号偏移)`，按出现顺序。
fn lex_static_blocks(source: &[u8]) -> Vec<(usize, usize)> {
    let mut scanner = Scanner::new(source, 0);
    let mut marks = Vec::new();
    while let Some((index, byte)) = scanner.next_code() {
        if byte == b's' && source[index..].starts_with(b"static") {
            let previous_ok = index == 0 || !is_word_byte(source[index - 1]);
            let next_ok =
                source.len() <= index + 6 || !is_word_byte(source[index + 6]);
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
    fn patch_targets_cover_every_browser_bundle_with_static_blocks() {
        assert_eq!(TARGETS.len(), 3);
        assert!(TARGETS.iter().all(|path| path.starts_with("node_modules/@deepseek-ai/")));
    }
}
