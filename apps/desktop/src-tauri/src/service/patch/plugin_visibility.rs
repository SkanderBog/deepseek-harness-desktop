use std::collections::BTreeSet;
use std::path::Path;

use crate::AppHandle;
use crate::service::plugin::{declared_packages, load_presets, profile_dir};
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const PLUGIN_PREFIX: &str = "dsh-tauri";

const PLUGIN_MANAGER_CLIENT_JS: &str =
    "node_modules/@deepseek-ai/dsh-client-ui-plugin-manager/lib/client.js";

const LISTED_PREFIX: &str = "const listed = state.packages.filter((pkg) => ";

const BUILTIN_GUARD: &str = "!BUILTIN_PROFILE_BUNDLES.has(pkg.name) && ";

const PREFIX_GUARD: &str = "!pkg.name.startsWith(\"dsh-tauri\") && ";

const MARKET_BUNDLE_FILES: [&str; 2] = [
    "node_modules/dshmarket/lib/profile.js",
    "node_modules/dshmarket/lib/order.js",
];

const INBOX_ANCHOR: &str = "export const INBOX_BUNDLES = new Set([";

fn patch_plugin_manager(source: &str) -> PatchOutcome {
    if source.contains(PREFIX_GUARD) {
        return PatchOutcome::AlreadyPatched;
    }
    let Some(start) = source.find(LISTED_PREFIX) else {
        return PatchOutcome::AnchorMissing;
    };
    let guard_start = start + LISTED_PREFIX.len();
    let Some(guard) = source[guard_start..].find(BUILTIN_GUARD) else {
        return PatchOutcome::AnchorMissing;
    };
    let insert_at = guard_start + guard + BUILTIN_GUARD.len();
    let mut patched = source.to_string();
    patched.insert_str(insert_at, PREFIX_GUARD);
    PatchOutcome::Patched(patched)
}

fn patch_inbox_bundles(source: &str, names: &[String]) -> PatchOutcome {
    let Some(start) = source.find(INBOX_ANCHOR) else {
        return PatchOutcome::AnchorMissing;
    };
    let body_start = start + INBOX_ANCHOR.len();
    let Some(end) = source[body_start..].find("])") else {
        return PatchOutcome::AnchorMissing;
    };
    let body = &source[body_start..body_start + end];
    let missing: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| !body.contains(&format!("'{name}'")))
        .collect();
    if missing.is_empty() {
        return PatchOutcome::AlreadyPatched;
    }

    if body.contains("//") {
        return PatchOutcome::AnchorMissing;
    }

    let multiline = body.contains('\n');
    let indent = if multiline {
        element_indent(source, body_start)
    } else {
        ""
    };
    let last = body.trim_end_matches([' ', '\t', '\r', '\n']);
    let separator = if last.is_empty() || last.ends_with(',') {
        ""
    } else {
        ","
    };
    let mut inserted = String::from(separator);
    for name in missing {
        if multiline {
            inserted.push('\n');
            inserted.push_str(indent);
        }
        inserted.push('\'');
        inserted.push_str(name);
        inserted.push_str("',");
    }
    let mut patched = source.to_string();
    patched.insert_str(body_start + last.len(), &inserted);
    PatchOutcome::Patched(patched)
}

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

fn leading_blank_len(text: &str) -> usize {
    text.len() - text.trim_start_matches([' ', '\t']).len()
}

fn is_safe_bundle_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '.' | '-' | '_'))
}

pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, PLUGIN_MANAGER_CLIENT_JS, patch_plugin_manager)?;
    let names = hidden_bundle_names(app_handle);
    if names.is_empty() {
        return Ok(());
    }
    let profile = profile_dir(app_handle);
    for file in MARKET_BUNDLE_FILES {
        patch_core_file(&profile, file, |source| patch_inbox_bundles(source, &names))?;
    }
    Ok(())
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, PLUGIN_MANAGER_CLIENT_JS, patch_plugin_manager)
}

fn hidden_bundle_names(app_handle: &AppHandle) -> Vec<String> {
    let internal = load_presets(app_handle)
        .into_iter()
        .filter(|plugin| plugin.internal)
        .map(|plugin| plugin.package.unwrap_or(plugin.id));
    let declared = declared_packages(app_handle).unwrap_or_default();
    internal
        .chain(declared)
        .filter(|name| name.starts_with(PLUGIN_PREFIX) && is_safe_bundle_name(name))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED_LINE: &str = "\t\t\tconst listed = state.packages.filter((pkg) => !BUILTIN_PROFILE_BUNDLES.has(pkg.name) && (pkg.installed || pkg.optional || pkg.error !== void 0));\n\t\t\tconst mine = listed.filter((pkg) => pkg.installed || !pkg.optional);\n";

    const LISTED_LINE_OFFICIAL: &str = "\t\t\tconst listed = state.packages.filter((pkg) => !BUILTIN_PROFILE_BUNDLES.has(pkg.name) && (pkg.installed || pkg.official || pkg.error !== void 0));\n\t\t\tconst mine = listed.filter((pkg) => !pkg.official);\n\t\t\tconst official = listed.filter((pkg) => pkg.official);\n";

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
    fn plugin_manager_hides_prefix_after_the_upstream_official_field() {
        match patch_plugin_manager(LISTED_LINE_OFFICIAL) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(
                    "!BUILTIN_PROFILE_BUNDLES.has(pkg.name) && !pkg.name.startsWith(\"dsh-tauri\") && (pkg.installed || pkg.official"
                ));
                assert!(patched.contains("const official = listed.filter((pkg) => pkg.official);"));
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
        let PatchOutcome::Patched(patched) = patch_plugin_manager(LISTED_LINE_OFFICIAL) else {
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
        assert_eq!(
            patch_inbox_bundles(&once, &names()),
            PatchOutcome::AlreadyPatched
        );
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

    #[test]
    fn market_lists_handle_single_line_literal_after_a_header() {
        let source =
            "// header line\nexport const INBOX_BUNDLES = new Set(['@deepseek-ai/dsh-base']);\n";
        match patch_inbox_bundles(source, &names()) {
            PatchOutcome::Patched(patched) => assert_eq!(
                patched,
                "// header line\nexport const INBOX_BUNDLES = new Set(['@deepseek-ai/dsh-base','dsh-tauri','dsh-tauri-ui',]);\n"
            ),
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn market_lists_handle_multi_line_literal_without_trailing_comma() {
        let source = "export const INBOX_BUNDLES = new Set([\n    '@deepseek-ai/dsh-base'\n]);\n";
        let PatchOutcome::Patched(patched) = patch_inbox_bundles(source, &names()) else {
            panic!("expected Patched");
        };
        assert_eq!(
            patched,
            "export const INBOX_BUNDLES = new Set([\n    '@deepseek-ai/dsh-base',\n    'dsh-tauri',\n    'dsh-tauri-ui',\n]);\n"
        );
        assert_eq!(
            patch_inbox_bundles(&patched, &names()),
            PatchOutcome::AlreadyPatched
        );
    }

    #[test]
    fn market_patch_skips_body_with_line_comment() {
        assert_eq!(
            patch_inbox_bundles(
                "export const INBOX_BUNDLES = new Set([\n    '@deepseek-ai/dsh-base' // note\n]);\n",
                &names()
            ),
            PatchOutcome::AnchorMissing
        );
    }

    #[test]
    fn bundle_names_must_be_plain_package_names() {
        assert!(is_safe_bundle_name("dsh-tauri-session"));
        assert!(is_safe_bundle_name("@scope/dsh-tauri.inner_v2"));
        assert!(!is_safe_bundle_name("dsh-tauri');evil('"));
        assert!(!is_safe_bundle_name("dsh-tauri\n"));
        assert!(!is_safe_bundle_name(""));
    }
}
