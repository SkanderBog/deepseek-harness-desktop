
use std::path::Path;

use crate::utils::{patch_core_file, patch_dsh, PatchOutcome};
use crate::{AppHandle};

const PATCH_MARKER: &str = "dsh-tauri-desktop: OpenCode Go session header";

const PI_AI_INDEX_JS: &str = "node_modules/@deepseek-ai/dsh-llm-pi-ai/lib/index.js";

const HEADERS_ANCHOR: &str =
    "function requestHeaders(headers) {\n\tconst attribution = attributionHeaders();";
const HEADERS_PATCHED: &str = r#"/* dsh-tauri-desktop: OpenCode Go session header */
function isOpenCodeRoute(provider, baseURL) {
	if (typeof provider === "string" && provider.toLowerCase().startsWith("opencode")) return true;
	if (typeof baseURL !== "string") return false;
	try {
		const host = new URL(baseURL).hostname.toLowerCase();
		return host === "opencode.ai" || host.endsWith(".opencode.ai");
	} catch {
		return false;
	}
}
function requestHeaders(headers, sessionId, provider, baseURL) {
	const attribution = {
		...attributionHeaders(),
		...(sessionId === void 0 || !isOpenCodeRoute(provider, baseURL) ? {} : { "x-deepseek-harness-session-id": String(sessionId) })
	};"#;

const CALL_ANCHOR: &str = "headers: requestHeaders(profile.headers)";
const CALL_PATCHED: &str =
    "headers: requestHeaders(profile.headers, options.sessionId, profile.provider, model.baseUrl)";

fn patch_source(source: &str) -> PatchOutcome {
    if source.contains(PATCH_MARKER) {
        return PatchOutcome::AlreadyPatched;
    }
    if !source.contains(HEADERS_ANCHOR) || !source.contains(CALL_ANCHOR) {
        return PatchOutcome::AnchorMissing;
    }
    let patched = source
        .replacen(HEADERS_ANCHOR, HEADERS_PATCHED, 1)
        .replacen(CALL_ANCHOR, CALL_PATCHED, 1);
    PatchOutcome::Patched(patched)
}

pub fn apply_at(core_dir: &Path) -> Result<(), String> {
    patch_core_file(core_dir, PI_AI_INDEX_JS, patch_source)
}
pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    patch_dsh(app_handle, PI_AI_INDEX_JS, patch_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        format!(
            "{HEADERS_ANCHOR}\n\tconst reserved = new Set(Object.keys(attribution).map((name) => name.toLowerCase()));\n\treturn {{ ...headers }};\n}}\n\t\t\t\t\t{CALL_ANCHOR}\n"
        )
    }

    #[test]
    fn patches_pristine_source_with_scoped_injection() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patch")
        };
        assert!(patched.contains(PATCH_MARKER));
        assert!(patched.contains("function isOpenCodeRoute(provider, baseURL)"));
        assert!(patched.contains("provider.toLowerCase().startsWith(\"opencode\")"));
        assert!(patched.contains("host === \"opencode.ai\" || host.endsWith(\".opencode.ai\")"));
        assert!(patched.contains("!isOpenCodeRoute(provider, baseURL)"));
        assert!(patched.contains(
            "requestHeaders(profile.headers, options.sessionId, profile.provider, model.baseUrl)"
        ));
        assert!(!patched.contains(CALL_ANCHOR));
    }

    #[test]
    fn patch_is_idempotent() {
        let PatchOutcome::Patched(patched) = patch_source(&fixture()) else {
            panic!("expected patch")
        };
        assert_eq!(patch_source(&patched), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn missing_headers_anchor_is_skipped() {
        assert_eq!(patch_source(CALL_ANCHOR), PatchOutcome::AnchorMissing);
    }

    #[test]
    fn missing_call_anchor_is_skipped() {
        assert_eq!(patch_source(HEADERS_ANCHOR), PatchOutcome::AnchorMissing);
    }
}
