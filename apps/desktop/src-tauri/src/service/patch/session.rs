
use std::path::Path;

use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};
use crate::{AppHandle};

const PATCH_MARKER: &str = "dsh-tauri-desktop: SessionStore.remove";
const ANCHOR: &str =
    "/** Remove one exact entered session and emit its paired disposal when announced. */";
const INSERTION: &str = r#"/** Remove one live session by id and run its official detach lifecycle. */
	remove(id) {
		const entry = this.store.get(id);
		if (entry === void 0) return false;
		entry.detach();
		return true;
	}
	/** Remove one exact entered session and emit its paired disposal when announced. */"#;

const SESSION_INDEX_JS: &str = "node_modules/@deepseek-ai/dsh-session/lib/index.js";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if !source.contains(ANCHOR) {
        return PatchOutcome::AnchorMissing;
    }
    PatchOutcome::Patched(source.replacen(ANCHOR, &format!("{INSERTION} /* {PATCH_MARKER} */"), 1))
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, SESSION_INDEX_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, SESSION_INDEX_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_remove_method() {
        let source = format!("{ANCHOR}\n");
        let PatchOutcome::Patched(patched) = patch_source(&source) else {
            panic!("expected patch")
        };
        assert!(patched.contains("remove(id)"));
        assert!(patched.contains(PATCH_MARKER));
    }

    #[test]
    fn patch_is_idempotent() {
        let source = format!("{ANCHOR}\n");
        let PatchOutcome::Patched(patched) = patch_source(&source) else {
            panic!("expected patch")
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }
}
