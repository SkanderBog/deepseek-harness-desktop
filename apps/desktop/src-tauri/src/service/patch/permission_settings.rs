use std::path::Path;

use crate::AppHandle;
use crate::utils::{PatchOutcome, patch_core_file, patch_dsh};

const PERMISSION_CLIENT_JS: &str =
    "node_modules/@deepseek-ai/dsh-client-ui-permission-presets/lib/client.js";
const SAVE_ERROR_ORIGINAL: &str = r#"saving = false;
			disposed = false;"#;
const SAVE_ERROR_PATCHED: &str = r#"saving = false;
			saveError = null;
			disposed = false;"#;
const RESPONSE_ORIGINAL: &str = r#"if (!response.ok) {
					this.fail(response.error);
					return;
				}
				this.describeFace.acceptView(response.value);"#;
const RESPONSE_PATCHED: &str = r#"if (!response.ok) {
					this.saveError = response.error instanceof Error ? response.error.message : String(response.error);
					this.fail(this.saveError);
					return;
				}
				this.saveError = null;
				this.describeFace.acceptView(response.value);"#;
const DERIVE_ORIGINAL: &str = r#"state.status = "ready";
						state.error = null;
						state.writable = writable;
						state.currentValue = resolved.currentValue;"#;
const DERIVE_V1: &str = r#"state.status = state.error === null ? "ready" : "error";
						state.writable = writable;
						state.currentValue = resolved.currentValue;"#;
const DERIVE_PATCHED: &str = r#"state.status = this.saveError === null ? "ready" : "error";
						state.error = this.saveError;
						state.writable = writable;
						state.currentValue = resolved.currentValue;"#;

fn patch_source(source: &str) -> PatchOutcome {
    let replacements = [
        (SAVE_ERROR_ORIGINAL, SAVE_ERROR_PATCHED),
        (RESPONSE_ORIGINAL, RESPONSE_PATCHED),
        (DERIVE_ORIGINAL, DERIVE_PATCHED),
    ];
    if replacements.iter().all(|(original, patched)| {
        source.matches(patched).count() == 1 && !source.contains(original)
    }) && !source.contains(DERIVE_V1)
    {
        return PatchOutcome::AlreadyPatched;
    }
    if source.matches(DERIVE_V1).count() > 1
        || (source.contains(DERIVE_V1) && source.contains(DERIVE_ORIGINAL))
        || replacements
            .iter()
            .any(|(_, patched)| source.contains(patched))
    {
        return PatchOutcome::AnchorMissing;
    }
    let source = source.replacen(DERIVE_V1, DERIVE_ORIGINAL, 1);
    if replacements
        .iter()
        .any(|(original, _)| source.matches(original).count() != 1)
    {
        return PatchOutcome::AnchorMissing;
    }
    let patched = replacements
        .iter()
        .fold(source, |source, (original, patched)| {
            source.replacen(original, patched, 1)
        });
    PatchOutcome::Patched(patched)
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, PERMISSION_CLIENT_JS, patch_source)
}

pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, PERMISSION_CLIENT_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> String {
        format!(
            "class Presets {{\n\t\tstate = {{}};\n\t\tasync save() {{\n\t\t\t{}\n\t\t}}\n\t\tasync load() {{\n\t\t\t{}\n\t\t}}\n\t\tderive() {{\n\t\t\t{}\n\t\t}}\n}}\n",
            SAVE_ERROR_ORIGINAL, RESPONSE_ORIGINAL, DERIVE_ORIGINAL
        )
    }

    #[test]
    fn patches_real_controller_and_upgrades_v1_once() {
        let legacy = controller();
        for source in [legacy.clone(), legacy.replace(DERIVE_ORIGINAL, DERIVE_V1)] {
            let PatchOutcome::Patched(patched) = patch_source(&source) else {
                panic!("expected Patched");
            };
            assert!(patched.contains(DERIVE_PATCHED));
            assert!(patched.contains(SAVE_ERROR_PATCHED));
            assert!(patched.contains(RESPONSE_PATCHED));
            assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
        }
    }

    #[test]
    fn leaves_changed_or_ambiguous_anchors_unchanged() {
        let legacy = controller();
        assert_eq!(patch_source(""), PatchOutcome::AnchorMissing);
        for anchor in [SAVE_ERROR_ORIGINAL, RESPONSE_ORIGINAL, DERIVE_ORIGINAL] {
            assert_eq!(
                patch_source(&legacy.replacen(anchor, "", 1)),
                PatchOutcome::AnchorMissing
            );
            assert_eq!(
                patch_source(&legacy.replacen(anchor, &anchor.repeat(2), 1)),
                PatchOutcome::AnchorMissing
            );
        }
        assert_eq!(
            patch_source(&legacy.replacen(DERIVE_ORIGINAL, &DERIVE_V1.repeat(2), 1)),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(
            patch_source(&format!("{legacy}{DERIVE_V1}")),
            PatchOutcome::AnchorMissing
        );
        assert_eq!(
            patch_source(&legacy.replacen(DERIVE_ORIGINAL, DERIVE_PATCHED, 1)),
            PatchOutcome::AnchorMissing
        );
    }
}
