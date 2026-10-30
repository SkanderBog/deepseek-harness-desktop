use std::path::Path;

use crate::AppHandle;
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const PATCH_MARKER: &str = "dsh-tauri: keep the model menu mounted through a WebKit blur";

const ONBLUR_ANCHOR: &str = "\t\t\t\tif (event.relatedTarget instanceof Node && (rootRef.current?.contains(event.relatedTarget) === true || menuRef.current?.contains(event.relatedTarget) === true)) return;";

const ONBLUR_PATCHED: &str = "\t\t\t\tif (event.relatedTarget === null || event.relatedTarget === void 0) return; /* dsh-tauri: keep the model menu mounted through a WebKit blur */\n\t\t\t\tif (event.relatedTarget instanceof Node && (rootRef.current?.contains(event.relatedTarget) === true || menuRef.current?.contains(event.relatedTarget) === true)) return;";

const MODEL_SELECTION_CLIENT_JS: &str =
    "node_modules/@deepseek-ai/dsh-client-ui-model-selection/lib/client.js";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if !source.contains(ONBLUR_ANCHOR) {
        return PatchOutcome::AnchorMissing;
    }
    PatchOutcome::Patched(source.replacen(ONBLUR_ANCHOR, ONBLUR_PATCHED, 1))
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, MODEL_SELECTION_CLIENT_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, MODEL_SELECTION_CLIENT_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        "\t\t\tconst onBlur = (event) => {\n\t\t\t\tif (event.relatedTarget instanceof Node && (rootRef.current?.contains(event.relatedTarget) === true || menuRef.current?.contains(event.relatedTarget) === true)) return;\n\t\t\t\tclose();\n\t\t\t};\n".into()
    }

    #[test]
    fn keeps_menu_mounted_when_related_target_is_missing() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patched source");
        };
        assert!(patched.contains(
            "if (event.relatedTarget === null || event.relatedTarget === void 0) return;"
        ));
        assert!(patched.contains(ONBLUR_ANCHOR));
        assert_eq!(patched.matches("close();").count(), 1);
    }

    #[test]
    fn patch_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected Patched");
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn skips_when_anchor_missing() {
        let rewritten = "\t\t\tconst onBlur = (event) => {\n\t\t\t\tif (event.relatedTarget === null) return;\n\t\t\t\tclose();\n\t\t\t};\n";
        assert_eq!(patch_source(rewritten), PatchOutcome::AnchorMissing);
        assert_eq!(patch_source(""), PatchOutcome::AnchorMissing);
    }
}
