
use semver::{Comparator, Op, Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DSH_PACKAGE: &str = "@deepseek-ai/dsh";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInspect {
    pub spec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub compatible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

pub fn is_dsh_package(name: &str) -> bool {
    name == DSH_PACKAGE || name.starts_with("@deepseek-ai/dsh-")
}

fn protocol_value(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    for prefix in ["workspace:", "catalog:", "portal:", "link:", "file:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if rest.is_empty() || matches!(rest, "*" | "^" | "~") {
                return None;
            }
            return Some(rest);
        }
    }
    Some(trimmed)
}

pub fn peers_from_manifest(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    let mut peers = BTreeMap::new();
    for section in ["dependencies", "peerDependencies"] {
        let Some(map) = manifest.get(section).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, raw) in map {
            if !is_dsh_package(name) {
                continue;
            }
            let Some(raw) = raw.as_str() else {
                continue;
            };
            let Some(value) = protocol_value(raw) else {
                continue;
            };
            peers.insert(name.clone(), value.to_string());
        }
    }
    peers
}

fn matches_including_prerelease(req: &VersionReq, version: &Version) -> bool {
    let matched = req.matches(version);
    if matched || version.pre.is_empty() {
        return matched;
    }
    let mut bridged = req.clone();
    bridged.comparators.push(Comparator {
        op: Op::Exact,
        major: version.major,
        minor: Some(version.minor),
        patch: Some(version.patch),
        pre: version.pre.clone(),
    });
    bridged.matches(version)
}

fn normalize_range(raw: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for token in raw.split_whitespace() {
        let token = token.trim_matches(',').trim();
        if token.is_empty() {
            continue;
        }
        if matches!(token, ">" | ">=" | "<" | "<=" | "=" | "^" | "~") {
            pending = Some(token.to_string());
            continue;
        }
        match pending.take() {
            Some(op) => parts.push(format!("{op}{token}")),
            None => match Version::parse(token.trim_start_matches('v')) {
                Ok(version) => parts.push(format!("={version}")),
                Err(_) => parts.push(token.to_string()),
            },
        }
    }
    if pending.is_some() || parts.is_empty() {
        return None;
    }
    Some(parts.join(","))
}

pub fn range_matches(raw: &str, runtime: &str) -> Option<bool> {
    let range = protocol_value(raw)?;
    let version = Version::parse(runtime.trim()).ok()?;
    let mut parsed_any = false;
    for alternative in range.split("||") {
        let Some(normalized) = normalize_range(alternative) else {
            continue;
        };
        let Ok(req) = VersionReq::parse(&normalized) else {
            continue;
        };
        parsed_any = true;
        if matches_including_prerelease(&req, &version) {
            return Some(true);
        }
    }
    if parsed_any {
        Some(false)
    } else {
        None
    }
}

pub fn evaluate(peers: &BTreeMap<String, String>, runtime: &str) -> Option<bool> {
    if peers.is_empty() {
        return None;
    }
    for raw in peers.values() {
        if range_matches(raw, runtime) == Some(false) {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn peers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, raw)| (name.to_string(), raw.to_string()))
            .collect()
    }

    #[test]
    fn workspace_protocol_is_normalized_before_parsing() {
        assert_eq!(range_matches("workspace:^0.2.0", "0.2.1"), Some(true));
        assert_eq!(range_matches("workspace:^0.2.0", "0.3.0"), Some(false));
        assert_eq!(range_matches("catalog:^0.2.0", "0.2.5"), Some(true));
        assert_eq!(
            range_matches("workspace:>=0.1.7-rc.1 <0.1.7-rc.5", "0.1.7-rc.3"),
            Some(true)
        );
    }

    #[test]
    fn prerelease_versions_match_across_patch_bumps() {
        assert_eq!(range_matches("^0.1.7-rc.1", "0.1.7-rc.2"), Some(true));
        assert_eq!(range_matches("^0.1.7-rc.1", "0.1.8-rc.1"), Some(true));
        assert_eq!(range_matches("^0.2.0-rc.1", "0.2.1-rc.1"), Some(true));
        assert_eq!(range_matches("^0.1.7", "0.1.7-rc.1"), Some(false));
    }

    #[test]
    fn bare_versions_are_exact_like_npm() {
        assert_eq!(range_matches("0.2.0", "0.2.0"), Some(true));
        assert_eq!(range_matches("0.2.0", "0.2.5"), Some(false));
        assert_eq!(range_matches("0.2.0", "0.3.0"), Some(false));
        assert_eq!(range_matches("=0.2.0", "0.2.5"), Some(false));
        assert_eq!(range_matches(">=0.2.0 <0.3.0", "0.2.5"), Some(true));
        assert_eq!(range_matches("^0.2.0", "0.2.5"), Some(true));
    }

    #[test]
    fn invalid_range_degrades_to_unknown_and_stays_compatible() {
        assert_eq!(range_matches("not a range", "0.2.0"), None);
        assert_eq!(range_matches(">>>1.0.0", "0.2.0"), None);
        assert_eq!(range_matches("", "0.2.0"), None);
        assert_eq!(range_matches("^1.0.0", "not-a-version"), None);
        assert_eq!(
            evaluate(&peers(&[("@deepseek-ai/dsh", "garbage")]), "0.2.0"),
            Some(true)
        );
    }

    #[test]
    fn alternatives_and_space_separated_comparators_are_supported() {
        assert_eq!(range_matches("^1.0.0 || ^0.2.0", "0.2.3"), Some(true));
        assert_eq!(range_matches("^1.0.0 || ^0.3.0", "0.2.3"), Some(false));
        assert_eq!(range_matches(">=0.1.0 <0.3.0", "0.2.0"), Some(true));
    }

    #[test]
    fn manifest_merges_dependencies_and_peer_dependencies_of_the_dsh_family() {
        let manifest = json!({
            "dependencies": {
                "@deepseek-ai/dsh-fs": "^0.1.0",
                "@deepseek-ai/dsh-llm": "^0.1.0",
                "dshmarket": "^1.0.0",
                "@deepseek-ai/other": "^1.0.0"
            },
            "peerDependencies": {
                "@deepseek-ai/dsh-llm": "^0.1.7-rc.1",
                "@deepseek-ai/dsh": "^0.2.0"
            }
        });
        let merged = peers_from_manifest(&manifest);
        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged.get("@deepseek-ai/dsh-llm").map(String::as_str),
            Some("^0.1.7-rc.1")
        );
        assert_eq!(
            merged.get("@deepseek-ai/dsh-fs").map(String::as_str),
            Some("^0.1.0")
        );
        assert_eq!(
            merged.get("@deepseek-ai/dsh").map(String::as_str),
            Some("^0.2.0")
        );
        assert!(!merged.contains_key("dshmarket"));
        assert!(!merged.contains_key("@deepseek-ai/other"));
        assert!(is_dsh_package("@deepseek-ai/dsh"));
        assert!(is_dsh_package("@deepseek-ai/dsh-llm"));
        assert!(!is_dsh_package("@deepseek-ai/dshx"));
    }

    #[test]
    fn evaluate_reports_unknown_without_family_peers() {
        assert_eq!(evaluate(&BTreeMap::new(), "0.2.0"), None);
        assert_eq!(
            evaluate(&peers(&[("@deepseek-ai/dsh", "^0.2.0")]), "0.2.0-rc.1"),
            Some(false)
        );
        assert_eq!(
            evaluate(
                &peers(&[
                    ("@deepseek-ai/dsh", "^0.2.0"),
                    ("@deepseek-ai/dsh-llm", "^0.2.0-rc.1"),
                ]),
                "0.2.0"
            ),
            Some(true)
        );
    }
}
