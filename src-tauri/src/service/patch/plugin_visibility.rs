//! 插件可见性补丁：`dsh-tauri-*` 在官方插件页与插件市场里不出现。
//!
//! 桌面端内置插件以 profile bundle 身份加载，因此会同时落进官方 Web 侧的两个界面：
//! 官方侧边栏 Plugins 管理页把 profile 持有的 bundle 列进 Installed 分组，插件市场
//! （dshmarket）把 profile 依赖里不属于 inbox 的包列成社区插件。两处都只看「名字在
//! 不在自己的内置/inbox 名单里」——上游没有可配置的隐藏开关（`docs/specs/plugin.baisc.md`
//! 的退级策略把「桌面壳补丁」列为第二级手段），故这里对两个前端做幂等文件补丁：
//!
//! - 官方 `dsh-client-ui-plugin-manager` 的内置名单是硬编码的
//!   `BUILTIN_PROFILE_BUNDLES`，唯一使用点是分组过滤行 `listed`（Installed/Official
//!   两个分组、卡片、详情页都从它派生）。把 `dsh-tauri-*` 与该名单同等对待即可整组消失；
//! - dshmarket 的 `INBOX_BUNDLES` 只有 `.has()` 调用点，名单本身是字面量数组，按壳的
//!   内置插件与当前 profile 里实际出现的 `dsh-tauri-*` 包名的并集补齐即可。
//!
//! 壳自身的插件弹窗（`get_dsh_plugins` → `src/ui/config/plugin.tsx`）不经过这两个
//! 界面，因此不受影响。挂点与其它补丁一致：`service::workflow::launch` 启动 dsh
//! 进程前，最佳努力、失败仅告警；锚点缺失（上游改写布局）时安全跳过。

use std::collections::BTreeSet;
use std::path::Path;

use tauri::AppHandle;

use crate::service::plugin::{declared_packages, load_presets, profile_dir};
use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};

/// 桌面端内置插件的包名前缀：`dsh-tauri-*` 一律视为「壳自己的插件」。
const PLUGIN_PREFIX: &str = "dsh-tauri";

/// 官方插件管理页前端 bundle（相对活动核心安装目录的包内路径）。
const PLUGIN_MANAGER_CLIENT_JS: &str =
    "node_modules/@deepseek-ai/dsh-client-ui-plugin-manager/lib/client.js";

/// 官方分组过滤行（不含前导缩进）：`Installed` / `Official` 分组的唯一来源。
const LISTED_FILTER: &str = "const listed = state.packages.filter((pkg) => !BUILTIN_PROFILE_BUNDLES.has(pkg.name) && (pkg.installed || pkg.optional || pkg.error !== void 0));";

/// 补丁后的同一行：把 `dsh-tauri-*` 与 `BUILTIN_PROFILE_BUNDLES` 同等对待。
const LISTED_FILTER_PATCHED: &str = "const listed = state.packages.filter((pkg) => !BUILTIN_PROFILE_BUNDLES.has(pkg.name) && !pkg.name.startsWith(\"dsh-tauri\") && (pkg.installed || pkg.optional || pkg.error !== void 0));";

/// 插件市场登记 inbox bundle 的两份编译产物（相对活动 profile 目录）：`routes.js`
/// 从 `profile.js` 取名单，校验逻辑从 `order.js` 取，两份都要补。
const MARKET_BUNDLE_FILES: [&str; 2] = [
    "node_modules/dshmarket/lib/profile.js",
    "node_modules/dshmarket/lib/order.js",
];

/// marketplace 名单字面量的锚点（两份产物里的写法一致）。
const INBOX_ANCHOR: &str = "export const INBOX_BUNDLES = new Set([";

/// 官方插件页补丁的纯函数部分：在分组过滤行里追加 `dsh-tauri-*` 判定。
///
/// 已插入判定即视为已打过（幂等）；上游改写该行时安全跳过——宁可就地失效，
/// 也不要在未知布局上盲插。
fn patch_plugin_manager(source: &str) -> PatchOutcome {
    if source.contains(LISTED_FILTER_PATCHED) {
        return PatchOutcome::AlreadyPatched;
    }
    if !source.contains(LISTED_FILTER) {
        return PatchOutcome::AnchorMissing;
    }
    PatchOutcome::Patched(source.replace(LISTED_FILTER, LISTED_FILTER_PATCHED))
}

/// 插件市场名单补丁的纯函数部分：把 `names` 里尚未登记的名字追加进
/// `INBOX_BUNDLES` 字面量。
///
/// 按名字逐个判定是否已存在，因此插件增减后再次启动会自动补齐（不做「整体已打过」
/// 的粗判，否则新增的 `dsh-tauri-*` 永远进不了名单）；名字全在名单里则返回
/// [`PatchOutcome::AlreadyPatched`]。找不到字面量时返回
/// [`PatchOutcome::AnchorMissing`]。
fn patch_inbox_bundles(source: &str, names: &[String]) -> PatchOutcome {
    let Some(start) = source.find(INBOX_ANCHOR) else {
        return PatchOutcome::AnchorMissing;
    };
    let body_start = start + INBOX_ANCHOR.len();
    let Some(end) = source[body_start..].find("])") else {
        return PatchOutcome::AnchorMissing;
    };
    let body_end = body_start + end;
    let missing: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| !source[body_start..body_end].contains(&format!("'{name}'")))
        .collect();
    if missing.is_empty() {
        return PatchOutcome::AlreadyPatched;
    }

    let before = &source[..body_end];
    let close_indent = trailing_blank(before);
    let insert_at = body_end - close_indent.len();
    let mut patched = source.to_string();
    if before.contains('\n') {
        let indent = element_indent(source, body_start);
        let mut inserted = String::new();
        for name in missing {
            inserted.push_str(indent);
            inserted.push('\'');
            inserted.push_str(name);
            inserted.push_str("',\n");
        }
        patched.insert_str(insert_at, &inserted);
    } else {
        let mut inserted = String::new();
        if !before.trim_end().ends_with(',') {
            inserted.push(',');
        }
        for name in missing {
            inserted.push('\'');
            inserted.push_str(name);
            inserted.push_str("',");
        }
        patched.insert_str(insert_at, &inserted);
    }
    PatchOutcome::Patched(patched)
}

/// `source` 从 `anchor` 之后第一个元素行的前导空白；用于让新元素与既有元素对齐。
fn element_indent(source: &str, anchor_end: usize) -> &str {
    let Some(offset) = source[anchor_end..].find('\n') else {
        return "";
    };
    let line_start = anchor_end + offset + 1;
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |end| line_start + end);
    &source[line_start..line_start + leading_blank_len(&source[line_start..line_end])]
}

/// 文本末尾的连续空白（换行前的对齐用：`])` 所在行的前导缩进）。
fn trailing_blank(text: &str) -> &str {
    &text[text.trim_end_matches([' ', '\t']).len()..]
}

/// 文本开头连续空白（空格/制表符）的字节数。
fn leading_blank_len(text: &str) -> usize {
    text.len() - text.trim_start_matches([' ', '\t']).len()
}

/// 把 `dsh-tauri-*` 从官方插件页与插件市场隐藏。
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, PLUGIN_MANAGER_CLIENT_JS, patch_plugin_manager)?;
    let names = hidden_bundle_names(app_handle);
    if names.is_empty() {
        return Ok(());
    }
    let profile = profile_dir(app_handle);
    for file in MARKET_BUNDLE_FILES {
        patch_core_file(&profile, file, |source| {
            patch_inbox_bundles(source, &names)
        })?;
    }
    Ok(())
}

/// 对显式给定的核心安装目录施加官方插件页补丁（E2E 编排复用）。
///
/// 市场补丁作用于 profile 目录、且依赖 marketplace 是否安装，不在此列。
pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, PLUGIN_MANAGER_CLIENT_JS, patch_plugin_manager)
}

/// 需要从官方插件页与插件市场隐藏的包名：壳的内置插件与当前 profile 里实际声明的
/// `dsh-tauri-*` 包名的并集（去重排序）。
///
/// 内置插件取自清单（debug 下含 `packages/*` 的发现结果）而非 profile，这样尚未装进
/// 档案、或正在自愈重装的新内置插件在第一次启动就被隐藏；profile 一侧覆盖早期以普通
/// 插件身份装进档案、已不在清单里的 `dsh-tauri-*`（例如 `dsh-tauri-session`）。任一侧
/// 读不到时退化为空，仍由另一侧决定名单。
fn hidden_bundle_names(app_handle: &AppHandle) -> Vec<String> {
    let internal = load_presets(app_handle)
        .into_iter()
        .filter(|plugin| plugin.internal)
        .map(|plugin| plugin.package.unwrap_or(plugin.id));
    let declared = declared_packages(app_handle).unwrap_or_default();
    internal
        .chain(declared)
        .filter(|name| name.starts_with(PLUGIN_PREFIX))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED_LINE: &str = "\t\t\tconst listed = state.packages.filter((pkg) => !BUILTIN_PROFILE_BUNDLES.has(pkg.name) && (pkg.installed || pkg.optional || pkg.error !== void 0));\n\t\t\tconst mine = listed.filter((pkg) => pkg.installed || !pkg.optional);\n";

    #[test]
    fn plugin_manager_hides_prefix_inside_group_filter() {
        match patch_plugin_manager(LISTED_LINE) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(
                    "!BUILTIN_PROFILE_BUNDLES.has(pkg.name) && !pkg.name.startsWith(\"dsh-tauri\") && (pkg.installed"
                ));
                assert!(patched.contains("const mine = listed.filter("));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn plugin_manager_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_plugin_manager(LISTED_LINE) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_plugin_manager(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn plugin_manager_skips_when_anchor_missing() {
        assert_eq!(
            patch_plugin_manager("\t\t\tconst listed = other.call();\n"),
            PatchOutcome::AnchorMissing
        );
    }

    /// 真实产物形态：单引号 + 四空格缩进 + 独立 `]);` 行。
    const INBOX: &str = "export const INBOX_BUNDLES = new Set([\n    '@deepseek-ai/dsh-base',\n    '@deepseek-ai/dsh-web-app',\n    '@deepseek-ai/dsh-headless',\n]);\n\nexport function readInstalled() {}\n";

    fn names() -> Vec<String> {
        vec!["dsh-tauri".to_string(), "dsh-tauri-ui".to_string()]
    }

    #[test]
    fn market_lists_are_extended_with_alignment() {
        match patch_inbox_bundles(INBOX, &names()) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(
                    "    '@deepseek-ai/dsh-headless',\n    'dsh-tauri',\n    'dsh-tauri-ui',\n]);"
                ));
                assert!(patched.ends_with("\n\nexport function readInstalled() {}\n"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn market_lists_append_only_missing_names() {
        let PatchOutcome::Patched(once) = patch_inbox_bundles(INBOX, &names()) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_inbox_bundles(&once, &names()), PatchOutcome::AlreadyPatched);
        match patch_inbox_bundles(&once, &["dsh-tauri-scheduler".to_string()]) {
            PatchOutcome::Patched(patched) => {
                assert_eq!(patched.matches("'dsh-tauri',").count(), 1);
                assert!(patched.contains("'dsh-tauri-scheduler',\n]);"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn market_lists_handle_single_line_literal() {
        let source = "export const INBOX_BUNDLES = new Set(['@deepseek-ai/dsh-base']);\n";
        match patch_inbox_bundles(source, &names()) {
            PatchOutcome::Patched(patched) => {
                assert_eq!(
                    patched,
                    "export const INBOX_BUNDLES = new Set(['@deepseek-ai/dsh-base','dsh-tauri','dsh-tauri-ui',]);\n"
                );
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn market_patch_skips_when_anchor_missing() {
        assert_eq!(
            patch_inbox_bundles("const OTHER = new Set([]);\n", &names()),
            PatchOutcome::AnchorMissing
        );
    }
}
