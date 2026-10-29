
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::is_package_name;

static PLUGIN_REF_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"failed to (?:apply|import) loader entry[^\n]*\(([^)]+)\)"#,
        r#"cannot resolve profile bundle\s+["']?([^"'\n]+)["']?"#,
        r#"profile bundle\s+["']?([^"'\n]+)["']?\s+declares no dsh\.bundle"#,
        r#"plugins? failed to load:\s*([A-Za-z0-9@/_.\-]+)"#,
        r#"([A-Za-z0-9@/_.\-]+):\s*pending\s*\(waiting for service:[^)]*\)"#,
        r#"([A-Za-z0-9@/_.\-]+):\s*(?:failed|import failed)"#,
    ]
    .iter()
    .map(|p| Regex::new(p).expect("static plugin ref pattern"))
    .collect()
});

static RE_FAILED_TO_LOAD_PLUGINS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Failed to load plugins\s*$").expect("literal"));

static RE_DUP_PREFIX_ROUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"duplicate prefix route\s+["']([^"']+)["']"#).expect("literal"));

pub(super) fn extract_plugin_refs(text: &str) -> Vec<String> {
    let mut refs = HashSet::new();
    for re in PLUGIN_REF_PATTERNS.iter() {
        for cap in re.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                let cand = m.as_str().trim();
                if is_package_name(cand) {
                    refs.insert(cand.to_string());
                }
            }
        }
    }
    for m in RE_FAILED_TO_LOAD_PLUGINS.find_iter(text) {
        let rest = &text[m.end()..];
        for line in rest.lines().take(12) {
            let cand = line.trim().trim_end_matches(['.', ',', ' ']);
            if is_package_name(cand) {
                refs.insert(cand.to_string());
            }
        }
    }
    refs.into_iter().collect()
}

pub(super) fn extract_duplicate_loader_entry(text: &str) -> Option<String> {
    let re = Regex::new(r#"duplicate loader entry id:\s*["']?([^"'\s]+)["']?"#).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

pub(super) fn extract_slot_conflict(text: &str) -> Option<String> {
    let re = Regex::new(r#"single slot\s+["']([^"']+)["']\s+already has a registration"#).ok()?;
    if let Some(c) = re.captures(text) {
        return c.get(1).map(|m| m.as_str().trim().to_string());
    }
    let re = Regex::new(r#"UI slot\s+["']([^"']+)["']\s+has duplicate registrations"#).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

pub(super) fn classify_reason(text: &str) -> (String, String) {
    if let Some(c) = RE_DUP_PREFIX_ROUTE.captures(text) {
        let route = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
        return ("duplicate_route".into(), route);
    }
    if let Some(entry) = extract_duplicate_loader_entry(text) {
        return ("duplicate_loader_entry".into(), entry);
    }
    if let Some(c) = PLUGIN_REF_PATTERNS[1].captures(text) {
        return (
            "cannot_resolve_bundle".into(),
            c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
        );
    }
    if text.contains("declares no dsh.bundle") {
        let pkg = extract_plugin_refs(text)
            .into_iter()
            .next()
            .unwrap_or_default();
        return ("no_dsh_bundle".into(), pkg);
    }
    if let Some(slot) = extract_slot_conflict(text) {
        return ("slot_conflict".into(), slot);
    }
    if text.contains("failed to import loader entry")
        || text.contains("failed to apply loader entry")
        || text.contains("did not activate")
    {
        return ("load_failed".into(), String::new());
    }
    ("unknown".into(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cannot_resolve_bundle_reuses_pattern_without_broadening_refs() {
        for (text, refs, reason, detail) in [
            (
                "cannot resolve profile bundle \"@scope/pkg\"",
                vec!["@scope/pkg"],
                "cannot_resolve_bundle",
                "@scope/pkg",
            ),
            (
                "cannot resolve profile bundle 'dsh-plugin'",
                vec!["dsh-plugin"],
                "cannot_resolve_bundle",
                "dsh-plugin",
            ),
            (
                "cannot resolve profile bundle foo/../../target",
                vec![],
                "cannot_resolve_bundle",
                "foo/../../target",
            ),
            ("cannot resolve profile bundle \"\"", vec![], "unknown", ""),
            (
                "cannot resolve profile bundle \"dsh-plugin\"\nduplicate loader entry id: alias",
                vec!["dsh-plugin"],
                "duplicate_loader_entry",
                "alias",
            ),
        ] {
            assert_eq!(extract_plugin_refs(text), refs, "{text}");
            assert_eq!(
                classify_reason(text),
                (reason.into(), detail.into()),
                "{text}"
            );
        }
    }

    #[test]
    fn extract_refs_from_failure_log() {
        let log = r#"
[stderr] failed to apply loader entry dshSidebarApi (@omdsh-dev/dsh-better-sidebar)
[stderr] cannot resolve profile bundle "dsh-web-ui-all"
"#;
        let refs = extract_plugin_refs(log);
        assert!(refs.contains(&"@omdsh-dev/dsh-better-sidebar".to_string()));
        assert!(refs.contains(&"dsh-web-ui-all".to_string()));
    }

    #[test]
    fn extract_refs_keeps_valid_official_leaf() {
        let log = r#"failed to import loader entry dshClientUi (@deepseek-ai/dsh-client-ui-chat)"#;
        let refs = extract_plugin_refs(log);

        assert_eq!(refs, vec!["@deepseek-ai/dsh-client-ui-chat"]);
    }

    #[test]
    fn extract_boot_card_keeps_valid_official_leaf() {
        let log = "Failed to load plugins\n@deepseek-ai/dsh-client-ui-chat\n";
        let refs = extract_plugin_refs(log);

        assert_eq!(refs, vec!["@deepseek-ai/dsh-client-ui-chat"]);
    }

    #[test]
    fn extract_refs_rejects_malformed_log_candidates() {
        let log = r#"
failed to apply loader entry badTraversal (foo/../../target)
cannot resolve profile bundle "@scope/pkg/extra"
profile bundle "@scope/.." declares no dsh.bundle
Failed to load plugins
@scope/.hidden
foo bar
"#;

        assert!(extract_plugin_refs(log).is_empty());
    }

    #[test]
    fn extract_refs_from_boot_card() {
        let log = "Failed to load plugins\ndsh-better-sidebar\n@scope/another\nAn unknown error occurred\n";
        let refs = extract_plugin_refs(log);
        assert!(refs.contains(&"dsh-better-sidebar".to_string()));
        assert!(refs.contains(&"@scope/another".to_string()));
        assert!(!refs.iter().any(|r| r.contains("unknown")));
    }

    #[test]
    fn extract_refs_from_pending_service_line() {
        let log = "Failed to load plugins\nweb boot: 1 entry did not activate\ndsh-tauri-rightclick: pending (waiting for service: remote.session)\n";
        let refs = extract_plugin_refs(log);
        assert_eq!(refs, vec!["dsh-tauri-rightclick"]);
    }

    #[test]
    fn extract_refs_from_pending_service_line_with_scope() {
        let log = "@scope/dsh-plugin-x: pending (waiting for service: uiRenderer)\n";
        let refs = extract_plugin_refs(log);
        assert_eq!(refs, vec!["@scope/dsh-plugin-x"]);
    }

    #[test]
    fn extract_refs_rejects_malformed_pending_line() {
        let log = "dsh-tauri-rightclick: pending (waiting for nothing)\n";
        assert!(extract_plugin_refs(log).is_empty());
        let log2 = "some package: pending without parentheses\n";
        assert!(extract_plugin_refs(log2).is_empty());
    }

    #[test]
    fn extract_refs_from_boot_entry_state_line() {
        let log = "Failed to load plugins dsh-office-kit web boot: 1 entry did not activate dsh-office-kit: failed";
        assert_eq!(extract_plugin_refs(log), vec!["dsh-office-kit"]);
        assert_eq!(classify_reason(log).0, "load_failed");

        let import_failed = "web boot: 1 entry did not activate\n@scope/dsh-plugin-x: import failed (see console for the import error)\n";
        assert_eq!(
            extract_plugin_refs(import_failed),
            vec!["@scope/dsh-plugin-x"]
        );
    }

    #[test]
    fn extract_duplicate_and_slot() {
        let route_log = r#"duplicate prefix route "/sidebar/api""#;
        assert_eq!(classify_reason(route_log).0, "duplicate_route");
        let entry_log = "duplicate loader entry id: \"dshSidebarApi\"";
        assert_eq!(
            extract_duplicate_loader_entry(entry_log).as_deref(),
            Some("dshSidebarApi")
        );
        let slot_log = r#"single slot "sidebar" already has a registration"#;
        assert_eq!(extract_slot_conflict(slot_log).as_deref(), Some("sidebar"));
    }
}
