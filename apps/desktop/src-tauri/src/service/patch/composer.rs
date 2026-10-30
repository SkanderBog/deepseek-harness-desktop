use std::path::Path;

use crate::AppHandle;
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const PATCH_MARKER: &str = "dsh-tauri: composer stays usable for a session outside every workspace";

const CAPABILITY_ATTRIBUTE: &str = "data-dsh-composer-cwd";

const INERT_ORIGINAL: &str = "const inert = sessionId === void 0 || hero && chipTitle === void 0;";

const INERT_PATCHED: &str =
    "const inert = sessionId === void 0 || hero && chipTitle === void 0 && cwd === void 0;";

const ANCHOR_KEYWORD: &str = "return module.exports;";

const CONVERSATION_CLIENT_JS: &str =
    "node_modules/@deepseek-ai/dsh-client-ui-conversation/lib/client.js";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    let needs_relax = if source.contains(INERT_ORIGINAL) {
        true
    } else if source.contains(INERT_PATCHED) {
        false
    } else {
        return PatchOutcome::AnchorMissing;
    };
    let Some((line_start, indent)) = locate_return_module_exports(source) else {
        return PatchOutcome::AnchorMissing;
    };

    let mut patched = source.to_string();
    patched.insert_str(
        line_start,
        &format!(
            "{indent}if (typeof document !== \"undefined\") document.documentElement.setAttribute(\"{CAPABILITY_ATTRIBUTE}\", \"1\"); /* {PATCH_MARKER} */\n"
        ),
    );
    if needs_relax {
        patched = patched.replacen(INERT_ORIGINAL, INERT_PATCHED, 1);
    }
    PatchOutcome::Patched(patched)
}

fn locate_return_module_exports(source: &str) -> Option<(usize, &str)> {
    let match_start = source.find(ANCHOR_KEYWORD)?;
    let line_start = source[..match_start]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let indent = &source[line_start..match_start];
    if !indent.chars().all(|c| c == '\t' || c == ' ') {
        return None;
    }
    Some((line_start, indent))
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, CONVERSATION_CLIENT_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, CONVERSATION_CLIENT_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        "\t\t\tconst inert = sessionId === void 0 || hero && chipTitle === void 0;\n\t\t\tconst heroWorkspaceRow = 1;\n\t\texports.apply = apply;\n\t\texports.inject = inject;\n\t\treturn module.exports;\n\t}\n});\n".into()
    }

    #[test]
    fn relaxes_inert_and_declares_capability() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected Patched, got {:?}", patch_source(&fixture()));
        };
        assert!(patched.contains(INERT_PATCHED));
        assert!(!patched.contains(INERT_ORIGINAL));
        assert!(patched.contains(&format!(
            "\t\tif (typeof document !== \"undefined\") document.documentElement.setAttribute(\"{CAPABILITY_ATTRIBUTE}\", \"1\"); /* {PATCH_MARKER} */\n\t\treturn module.exports;\n"
        )));
    }

    #[test]
    fn patch_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn upstream_fix_only_declares_capability() {
        let fixed = format!("\t\t\t{INERT_PATCHED}\n\t\treturn module.exports;\n");
        let PatchOutcome::Patched(patched) = patch_source(&fixed) else {
            panic!("expected Patched");
        };
        assert!(patched.contains(INERT_PATCHED));
        assert!(patched.contains(CAPABILITY_ATTRIBUTE));
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn loose_substring_is_not_treated_as_upstream_fix() {
        let source = format!(
            "\t\t\t// 说明：{INERT_PATCHED}\n\t\t\t{INERT_ORIGINAL}\n\t\treturn module.exports;\n"
        );
        let PatchOutcome::Patched(patched) = patch_source(&source) else {
            panic!("expected Patched");
        };
        assert!(!patched.contains(&format!("{INERT_ORIGINAL}\n")));
        assert_eq!(patched.matches(INERT_PATCHED).count(), 2);
    }

    #[test]
    fn skips_when_anchor_missing() {
        assert_eq!(
            patch_source("\t\t\tconst inert = 1;\n"),
            PatchOutcome::AnchorMissing
        );
        let no_return = format!("\t\t\t{INERT_ORIGINAL}\n");
        assert_eq!(patch_source(&no_return), PatchOutcome::AnchorMissing);
    }
}
