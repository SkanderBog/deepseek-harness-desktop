use std::path::Path;

use crate::AppHandle;
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const ANCHOR_KEYWORD: &str = "return module.exports;";

const RENDERER_CLIENT_JS: &str = "node_modules/@deepseek-ai/dsh-client-ui-renderer/lib/client.js";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains("exports.SlotOutlet") {
        return PatchOutcome::AlreadyPatched;
    }
    let Some((line_start, indent)) = locate_return_module_exports(source) else {
        return PatchOutcome::AnchorMissing;
    };
    let mut patched = source.to_string();
    patched.insert_str(
        line_start,
        &format!("{indent}exports.SlotOutlet = SlotOutlet;\n"),
    );
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
    patch_core_file(core_dir, RENDERER_CLIENT_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, RENDERER_CLIENT_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAIL: &str = "\t\t//#endregion\n\t\texports.apply = apply;\n\t\texports.inject = inject;\n\treturn module.exports;\n\t}\n});\n";

    const TAIL_DOUBLE_TAB: &str = "\t\t//#endregion\n\t\texports.apply = apply;\n\t\texports.inject = inject;\n\t\treturn module.exports;\n\t}\n});\n";

    #[test]
    fn patch_inserts_export_before_return() {
        match patch_source(TAIL) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains("exports.SlotOutlet = SlotOutlet;"));
                assert!(patched.contains(
                    "\t\texports.inject = inject;\n\texports.SlotOutlet = SlotOutlet;\n\treturn module.exports;\n"
                ));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn patch_matches_double_tab_indentation() {
        match patch_source(TAIL_DOUBLE_TAB) {
            PatchOutcome::Patched(patched) => {
                assert!(patched.contains(
                    "\t\texports.inject = inject;\n\t\texports.SlotOutlet = SlotOutlet;\n\t\treturn module.exports;\n"
                ));
                assert!(!patched.contains("\n\texports.SlotOutlet = SlotOutlet;\n\treturn"));
            }
            other => panic!("expected Patched, got {other:?}"),
        }
    }

    #[test]
    fn patch_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_source(TAIL) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn patch_skips_when_anchor_missing() {
        let altered = "\t\texports.apply = apply;\n\t\texports.inject = inject;\n";
        assert_eq!(patch_source(altered), PatchOutcome::AnchorMissing);
    }
}
