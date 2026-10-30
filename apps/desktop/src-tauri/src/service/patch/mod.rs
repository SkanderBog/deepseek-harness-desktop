pub(crate) mod chat_follow_anchor;
pub(crate) mod composer;
pub(crate) mod conversation_draft;
pub(crate) mod llm_session;
pub(crate) mod mcp_startup;
pub(crate) mod mobile_composer;
pub(crate) mod mobile_sidebar;
pub(crate) mod model_selection;
pub(crate) mod permission_settings;
pub(crate) mod pi_ai_thinking;
pub(crate) mod plugin_visibility;
pub(crate) mod renderer;
pub(crate) mod scroll_follow;
pub(crate) mod session;
pub(crate) mod workspace;
pub(crate) mod workspace_view;

use std::path::Path;

pub(crate) fn apply_all_at(core_dir: &Path) -> Result<(), String> {
    #[allow(clippy::type_complexity)]
    let patches: [(&str, fn(&Path) -> Result<(), String>); 16] = [
        ("renderer", renderer::apply_at),
        ("mcp_startup", mcp_startup::apply_at),
        ("composer", composer::apply_at),
        ("conversation_draft", conversation_draft::apply_at),
        ("permission_settings", permission_settings::apply_at),
        ("scroll_follow", scroll_follow::apply_at),
        ("chat_follow_anchor", chat_follow_anchor::apply_at),
        ("mobile_composer", mobile_composer::apply_at),
        ("mobile_sidebar", mobile_sidebar::apply_at),
        ("session", session::apply_at),
        ("llm_session", llm_session::apply_at),
        ("pi_ai_thinking", pi_ai_thinking::apply_at),
        ("model_selection", model_selection::apply_at),
        ("workspace", workspace::apply_at),
        ("workspace_view", workspace_view::apply_at),
        ("plugin_visibility", plugin_visibility::apply_at),
    ];
    let mut failures = Vec::new();
    for (name, apply) in patches {
        if let Err(e) = apply(core_dir) {
            if e.contains("DSH_PATCH_") {
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}
