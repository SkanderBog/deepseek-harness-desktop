
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use semver::Version as Semver;
use serde::Deserialize;
use serde_json::Value;

use super::installed::profile_dir;
use super::watch::DshPlugin;
use crate::{AppHandle};

const UPDATES_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub update_available: bool,
    pub latest: Option<String>,
}

struct CacheEntry {
    info: UpdateInfo,
    at: Instant,
}

static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(
    id: &str,
    spec: &str,
    version: &str,
    locked: &HashMap<String, String>,
    proxy_url: &str,
) -> String {
    let locked_commit = extract_github_target(spec)
        .and_then(|_| locked.get(id))
        .map(String::as_str)
        .unwrap_or_default();
    format!("{id}\u{0}{proxy_url}\u{0}{spec}\u{0}{version}\u{0}{locked_commit}")
}

pub(crate) fn known_latest(app_handle: &AppHandle, id: &str) -> Option<String> {
    let proxy_url = crate::config::get_store_dat_setting(app_handle).proxy_url;
    cached_latest(&cache().lock().unwrap(), id, &proxy_url)
}

fn cached_latest(cache: &HashMap<String, CacheEntry>, id: &str, proxy_url: &str) -> Option<String> {
    let prefix = format!("{id}\u{0}{proxy_url}\u{0}");
    cache
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .max_by_key(|(_, entry)| entry.at)
        .and_then(|(_, entry)| entry.info.latest.clone())
}

fn effective_spec(profile: &Path, id: &str, spec: &str) -> String {
    catalog_spec(profile, id, spec).unwrap_or_else(|| spec.to_string())
}

fn catalog_spec(profile: &Path, id: &str, spec: &str) -> Option<String> {
    let name = spec.strip_prefix("catalog:")?;
    let text = std::fs::read_to_string(profile.join("pnpm-workspace.yaml")).ok()?;
    let workspace: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let entries = if name.is_empty() {
        workspace.get("catalog")?
    } else {
        workspace.get("catalogs")?.get(name)?
    };
    entries.get(id)?.as_str().map(String::from)
}

fn read_specs(app_handle: &AppHandle) -> HashMap<String, String> {
    let dir = profile_dir(app_handle);
    let Ok(content) = std::fs::read_to_string(dir.join("package.json")) else {
        return HashMap::new();
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&content) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    if let Some(deps) = manifest.get("dependencies").and_then(Value::as_object) {
        for (name, spec) in deps {
            if let Some(s) = spec.as_str() {
                out.insert(name.clone(), effective_spec(&dir, name, s));
            }
        }
    }
    out
}

fn read_locked_commits(profile: &Path, specs: &HashMap<String, String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(profile.join("pnpm-lock.yaml")) else {
        return out;
    };
    let re =
        regex::Regex::new(r"codeload\.github\.com/([^/\s]+)/([^/\s]+)/tar\.gz/([0-9a-fA-F]{7,40})")
            .expect("static codeload regex");
    let mut has_project_dependencies = false;
    for document in serde_yaml::Deserializer::from_str(&text) {
        let Ok(lockfile) = serde_yaml::Value::deserialize(document) else {
            return HashMap::new();
        };
        let Some(current_importer) = lockfile.get("importers").and_then(|value| value.get("."))
        else {
            continue;
        };
        let Some(dependencies) = current_importer
            .get("dependencies")
            .and_then(serde_yaml::Value::as_mapping)
        else {
            continue;
        };
        if std::mem::replace(&mut has_project_dependencies, true) {
            return HashMap::new();
        }
        for (id, dependency) in dependencies {
            let Some(id) = id.as_str() else {
                continue;
            };
            let Some(version) = dependency
                .get("version")
                .and_then(serde_yaml::Value::as_str)
            else {
                continue;
            };
            let Some(cap) = re.captures(version) else {
                continue;
            };
            let Some(expected_target) = specs.get(id).and_then(|spec| extract_github_target(spec))
            else {
                continue;
            };
            let resolved_repo = format!("{}/{}", &cap[1], &cap[2]);
            if !resolved_repo.eq_ignore_ascii_case(&expected_target.repo) {
                continue;
            }
            out.insert(id.to_string(), cap[3].to_ascii_lowercase());
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GitReference {
    Head,
    Named(String),
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubTarget {
    repo: String,
    reference: GitReference,
}

fn extract_github_target(spec: &str) -> Option<GitHubTarget> {
    if let Some(rest) = spec.strip_prefix("github:") {
        let (path, reference) = split_reference(rest);
        let path = path.trim_end_matches('/');
        let path = path
            .strip_suffix(".git")
            .unwrap_or(path)
            .trim_end_matches('/');
        return is_owner_repo(path).then(|| GitHubTarget {
            repo: path.to_string(),
            reference,
        });
    }

    if let Ok(url) = reqwest::Url::parse(spec) {
        match url.host_str() {
            Some(host)
                if host.eq_ignore_ascii_case("codeload.github.com")
                    && url.scheme().eq_ignore_ascii_case("https") =>
            {
                return codeload_target_from_path(url.path());
            }
            Some(host)
                if host.eq_ignore_ascii_case("github.com")
                    && is_supported_git_scheme(url.scheme()) =>
            {
                let reference = url
                    .fragment()
                    .map(parse_reference)
                    .unwrap_or(GitReference::Head);
                return github_target_from_path(url.path(), reference);
            }
            _ => {}
        }
    }

    let after = strip_prefix_ascii_case(spec, "git+ssh://git@github.com:")
        .or_else(|| strip_prefix_ascii_case(spec, "git@github.com:"))?;
    let after = strip_optional_ssh_port(after);
    let (path, reference) = split_reference(after);
    github_target_from_path(path, reference)
}

fn is_supported_git_scheme(scheme: &str) -> bool {
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "git" | "http" | "https" | "git+http" | "git+https" | "git+ssh" | "ssh"
    )
}

fn codeload_target_from_path(path: &str) -> Option<GitHubTarget> {
    let mut parts = path.trim_matches('/').split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "tar.gz" {
        return None;
    }
    let archive_target = parts.collect::<Vec<_>>().join("/");
    let repo = format!("{owner}/{repo}");
    (is_owner_repo(&repo) && !archive_target.is_empty()).then_some(GitHubTarget {
        repo,
        reference: GitReference::Head,
    })
}

fn github_target_from_path(path: &str, reference: GitReference) -> Option<GitHubTarget> {
    let path = path.split('?').next().unwrap_or(path).trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let repo = format!("{owner}/{repo}");
    is_owner_repo(&repo).then_some(GitHubTarget { repo, reference })
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

fn strip_optional_ssh_port(value: &str) -> &str {
    let Some((candidate, rest)) = value.split_once(':') else {
        return value;
    };
    if !candidate.is_empty() && candidate.bytes().all(|byte| byte.is_ascii_digit()) {
        rest
    } else {
        value
    }
}

fn split_reference(value: &str) -> (&str, GitReference) {
    match value.split_once('#') {
        Some((path, selectors)) => (path, parse_reference(selectors)),
        None => (value, GitReference::Head),
    }
}

fn parse_reference(selectors: &str) -> GitReference {
    let Some(decoded) = percent_decode_once(selectors) else {
        return GitReference::Unsupported;
    };
    if decoded.is_empty() {
        return GitReference::Head;
    }
    let mut named = None;
    let mut has_semver = false;
    for selector in decoded.split('&') {
        if selector.is_empty() {
            return GitReference::Unsupported;
        }
        if selector.starts_with("path:") {
            continue;
        }
        if selector.starts_with("semver:") {
            has_semver = true;
            continue;
        }
        named = Some(selector.to_string());
    }
    match named {
        _ if has_semver => GitReference::Unsupported,
        Some(reference) => GitReference::Named(reference),
        None => GitReference::Head,
    }
}

fn percent_decode_once(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let high = *bytes.get(index + 1)?;
        let low = *bytes.get(index + 2)?;
        decoded.push((hex_value(high)? << 4) | hex_value(low)?);
        index += 3;
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    char::from(value).to_digit(16).map(|digit| digit as u8)
}

fn is_owner_repo(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !repo.is_empty()
        && !value.contains('@')
        && owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_upgrade(installed: &str, latest: &str) -> bool {
    match (Semver::parse(installed), Semver::parse(latest)) {
        (Ok(i), Ok(l)) => l > i,
        _ => false,
    }
}

pub(crate) fn encode_registry_name(name: &str) -> String {
    name.replace('@', "%40").replace('/', "%2F")
}

pub(crate) async fn fetch_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let res = client
        .get(url)
        .header("accept", "application/json")
        .header("user-agent", "deepseek-harness-desktop")
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Value>().await.ok()
}

fn github_commit_url(target: &GitHubTarget) -> Option<reqwest::Url> {
    let (owner, repo) = target.repo.split_once('/')?;
    let reference = match &target.reference {
        GitReference::Head => "HEAD",
        GitReference::Named(reference) => reference,
        GitReference::Unsupported => return None,
    };
    let mut url = reqwest::Url::parse("https://api.github.com").ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.extend(["repos", owner, repo, "commits"]);
        segments.push(reference);
    }
    Some(url)
}

fn immutable_commit(reference: &str) -> Option<String> {
    (reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| reference.to_ascii_lowercase())
}

async fn fetch_target_sha(client: &reqwest::Client, target: &GitHubTarget) -> Option<String> {
    if let GitReference::Named(reference) = &target.reference {
        if let Some(commit) = immutable_commit(reference) {
            return Some(commit);
        }
    }
    let url = github_commit_url(target)?;
    let v = fetch_json(client, url.as_str()).await?;
    v.get("sha")?.as_str().map(String::from)
}

async fn fetch_npm_latest(client: &reqwest::Client, name: &str) -> Option<String> {
    let url = format!(
        "https://registry.npmjs.org/{}/latest",
        encode_registry_name(name)
    );
    let v = fetch_json(client, &url).await?;
    v.get("version")?.as_str().map(String::from)
}

async fn compute_update(
    client: &reqwest::Client,
    id: &str,
    spec: &str,
    installed_version: Option<&str>,
    locked: &HashMap<String, String>,
) -> UpdateInfo {
    if spec.starts_with("link:") || spec.starts_with("file:") {
        return UpdateInfo {
            update_available: false,
            latest: None,
        };
    }

    if let Some(target) = extract_github_target(spec) {
        let current = locked.get(id).map(|commit| commit.to_ascii_lowercase());
        let latest = fetch_target_sha(client, &target).await;
        return UpdateInfo {
            update_available: current.is_some() && latest.is_some() && current != latest,
            latest,
        };
    }

    let latest = fetch_npm_latest(client, id).await;
    let update_available = match (installed_version, latest.as_deref()) {
        (Some(i), Some(l)) => is_upgrade(i, l),
        _ => false,
    };
    UpdateInfo {
        update_available,
        latest,
    }
}

pub fn apply_cache(app_handle: &AppHandle, plugins: &mut [DshPlugin]) {
    let specs = read_specs(app_handle);
    let locked = read_locked_commits(&profile_dir(app_handle), &specs);
    let proxy_url = crate::config::get_store_dat_setting(app_handle).proxy_url;
    let cache = cache().lock().unwrap();
    let now = Instant::now();
    for p in plugins.iter_mut() {
        let spec = specs.get(&p.id).cloned().unwrap_or_default();
        let key = cache_key(&p.id, &spec, &p.version, &locked, &proxy_url);
        if let Some(entry) = cache.get(&key) {
            if now.duration_since(entry.at) < UPDATES_TTL {
                p.update_available = entry.info.update_available;
                p.latest_version = entry.info.latest.clone();
            }
        }
    }
}

pub async fn refresh(app_handle: &AppHandle) -> Result<Vec<DshPlugin>, String> {
    let mut plugins = super::watch::list(app_handle);
    let specs = read_specs(app_handle);
    let locked = read_locked_commits(&profile_dir(app_handle), &specs);
    let proxy_url = crate::config::get_store_dat_setting(app_handle).proxy_url;
    let client = crate::config::proxy::client_builder(&proxy_url)?
        .user_agent("deepseek-harness-desktop")
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("UPDATES_CLIENT: {e}"))?;

    struct Task {
        idx: usize,
        key: String,
        id: String,
        spec: String,
        version: Option<String>,
    }

    let now = Instant::now();
    let mut tasks: Vec<Task> = Vec::new();
    {
        let cache = cache().lock().unwrap();
        for (idx, p) in plugins.iter_mut().enumerate() {
            let spec = specs.get(&p.id).cloned().unwrap_or_default();
            let key = cache_key(&p.id, &spec, &p.version, &locked, &proxy_url);
            if let Some(entry) = cache.get(&key) {
                if now.duration_since(entry.at) < UPDATES_TTL {
                    p.update_available = entry.info.update_available;
                    p.latest_version = entry.info.latest.clone();
                    continue;
                }
            }
            tasks.push(Task {
                idx,
                key,
                id: p.id.clone(),
                spec,
                version: (!p.version.is_empty()).then(|| p.version.clone()),
            });
        }
    }

    let results = futures_util::future::join_all(tasks.into_iter().map(|t| {
        let c = &client;
        let lock = &locked;
        async move {
            let info = compute_update(c, &t.id, &t.spec, t.version.as_deref(), lock).await;
            (t.idx, t.key, info)
        }
    }))
    .await;

    let mut cache = cache().lock().unwrap();
    for (idx, key, info) in results {
        if let Some(p) = plugins.get_mut(idx) {
            p.update_available = info.update_available;
            p.latest_version = info.latest.clone();
        }
        cache.insert(
            key,
            CacheEntry {
                info,
                at: Instant::now(),
            },
        );
    }

    Ok(plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_update_cache_is_not_reused_after_proxy_changes() {
        let locked = HashMap::new();
        for (before, after) in [
            ("", "http://127.0.0.1:7897/"),
            ("http://127.0.0.1:7897/", "socks5h://127.0.0.1:1080"),
            ("http://127.0.0.1:7897/", ""),
            (
                "http://user:old@proxy.example/",
                "http://user:new@proxy.example/",
            ),
        ] {
            let entries = HashMap::from([(
                cache_key("plugin", "^1.0.0", "1.0.0", &locked, before),
                CacheEntry {
                    info: UpdateInfo {
                        update_available: false,
                        latest: None,
                    },
                    at: Instant::now(),
                },
            )]);
            let key = cache_key("plugin", "^1.0.0", "1.0.0", &locked, after);
            assert!(
                !entries.contains_key(&key),
                "proxy change reused a cached failure"
            );
        }
    }

    #[test]
    fn normalized_equivalent_proxies_reuse_update_cache() {
        let locked = HashMap::new();
        let before =
            crate::config::proxy::normalize_proxy_url(" HTTP://proxy.example:80 ").unwrap();
        let after = crate::config::proxy::normalize_proxy_url("http://proxy.example/").unwrap();
        let entries = HashMap::from([(
            cache_key("plugin", "^1.0.0", "1.0.0", &locked, &before),
            CacheEntry {
                info: UpdateInfo {
                    update_available: true,
                    latest: Some("2.0.0".into()),
                },
                at: Instant::now(),
            },
        )]);
        let key = cache_key("plugin", "^1.0.0", "1.0.0", &locked, &after);
        let entry = entries
            .get(&key)
            .expect("unchanged proxy lost its cached result");
        assert!(entry.info.update_available);
        assert_eq!(entry.info.latest.as_deref(), Some("2.0.0"));
        assert_eq!(
            cached_latest(&entries, "plugin", &after).as_deref(),
            Some("2.0.0")
        );
    }

    #[test]
    fn late_old_proxy_result_does_not_replace_current_update_or_install_target() {
        let locked = HashMap::new();
        let proxy = "http://127.0.0.1:7897/";
        let key = cache_key("plugin", "^1.0.0", "1.0.0", &locked, proxy);
        let now = Instant::now();
        let mut entries = HashMap::from([(
            key.clone(),
            CacheEntry {
                info: UpdateInfo {
                    update_available: true,
                    latest: Some("2.0.0".into()),
                },
                at: now,
            },
        )]);
        entries.insert(
            cache_key("plugin", "^1.0.0", "1.0.0", &locked, ""),
            CacheEntry {
                info: UpdateInfo {
                    update_available: false,
                    latest: Some("1.0.0".into()),
                },
                at: now + Duration::from_millis(1),
            },
        );
        let entry = entries.get(&key).unwrap();
        assert!(entry.info.update_available);
        assert_eq!(entry.info.latest.as_deref(), Some("2.0.0"));
        assert_eq!(
            cached_latest(&entries, "plugin", proxy).as_deref(),
            Some("2.0.0")
        );
    }

    #[test]
    fn cached_install_target_ignores_another_proxy() {
        let entries = HashMap::from([(
            cache_key("plugin", "^1.0.0", "1.0.0", &HashMap::new(), ""),
            CacheEntry {
                info: UpdateInfo {
                    update_available: true,
                    latest: Some("2.0.0".into()),
                },
                at: Instant::now(),
            },
        )]);
        assert_eq!(
            cached_latest(&entries, "plugin", "http://127.0.0.1:7897/"),
            None
        );
        assert_eq!(cached_latest(&entries, "different-plugin", ""), None);
    }

    #[test]
    fn hex_value_accepts_only_ascii_hex_nibbles() {
        for byte in 0..=u8::MAX {
            let expected = b"0123456789abcdef"
                .iter()
                .position(|candidate| *candidate == byte.to_ascii_lowercase())
                .map(|position| position as u8);
            assert_eq!(hex_value(byte), expected, "byte={byte}");
        }
    }

    #[test]
    fn percent_decode_preserves_single_pass_and_rejects_invalid_escapes() {
        for (encoded, expected) in [
            ("branch%2Ffeature", Some("branch/feature")),
            ("%41%4a%4F", Some("AJO")),
            ("%252F", Some("%2F")),
            ("%E4%B8%AD", Some("中")),
            ("%", None),
            ("%2", None),
            ("%2G", None),
            ("%FF", None),
        ] {
            assert_eq!(
                percent_decode_once(encoded).as_deref(),
                expected,
                "{encoded}"
            );
        }
    }

    fn target(repo: &str, reference: Option<&str>) -> Option<GitHubTarget> {
        Some(GitHubTarget {
            repo: repo.to_string(),
            reference: reference
                .map(|reference| GitReference::Named(reference.to_string()))
                .unwrap_or(GitReference::Head),
        })
    }

    fn unsupported_target(repo: &str) -> Option<GitHubTarget> {
        Some(GitHubTarget {
            repo: repo.to_string(),
            reference: GitReference::Unsupported,
        })
    }

    fn workspace_probe(label: &str, yaml: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-update-cat-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-workspace.yaml"), yaml).unwrap();
        dir
    }

    #[test]
    fn effective_spec_resolves_default_and_named_catalogs() {
        let dir = workspace_probe(
            "resolve",
            "packages:\n  - .\ncatalog:\n  dsh-better-sidebar: ^0.18.1\ncatalogs:\n  next:\n    dsh-better-sidebar: ^0.19.0\n",
        );
        assert_eq!(
            effective_spec(&dir, "dsh-better-sidebar", "catalog:"),
            "^0.18.1"
        );
        assert_eq!(
            effective_spec(&dir, "dsh-better-sidebar", "catalog:next"),
            "^0.19.0"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn effective_spec_passes_through_when_it_cannot_resolve() {
        let dir = workspace_probe(
            "passthrough",
            "packages:\n  - .\ncatalog:\n  other: ^1.0.0\n",
        );
        assert_eq!(effective_spec(&dir, "dsh-probe", "^2.0.0"), "^2.0.0");
        assert_eq!(
            effective_spec(&dir, "dsh-probe", "github:a/b"),
            "github:a/b"
        );
        assert_eq!(effective_spec(&dir, "dsh-probe", "catalog:"), "catalog:");
        assert_eq!(
            effective_spec(&dir, "dsh-probe", "catalog:missing"),
            "catalog:missing"
        );

        let absent =
            std::env::temp_dir().join(format!("dsh-update-cat-{}-absent", std::process::id()));
        let _ = std::fs::remove_dir_all(&absent);
        assert_eq!(effective_spec(&absent, "dsh-probe", "catalog:"), "catalog:");

        let broken = workspace_probe("broken", "catalog: [not: a mapping\n");
        assert_eq!(effective_spec(&broken, "dsh-probe", "catalog:"), "catalog:");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&broken).ok();
    }

    #[test]
    fn effective_spec_exposes_the_git_target_of_a_catalog_entry() {
        let sha = "b3a69187e1bac1bf6162e3d37d005e58bc2ee74e";
        let dir = workspace_probe(
            "git",
            &format!(
                "catalog:\n  billion-context-dsh: github:Tyan66666/billion-context-dsh#{sha}\n"
            ),
        );
        assert_eq!(extract_github_target("catalog:"), None);
        let spec = effective_spec(&dir, "billion-context-dsh", "catalog:");
        assert_eq!(
            extract_github_target(&spec),
            target("Tyan66666/billion-context-dsh", Some(sha))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upgrade_requires_strictly_newer() {
        assert!(is_upgrade("1.0.0", "1.0.1"));
        assert!(is_upgrade("1.0.0", "2.0.0"));
        assert!(!is_upgrade("1.0.1", "1.0.0"));
        assert!(!is_upgrade("1.0.0", "1.0.0"));
        assert!(!is_upgrade("1.0.0", "canary"));
        assert!(!is_upgrade("0.0.0", "0.0.0"));
    }

    #[test]
    fn upgrade_handles_prerelease() {
        assert!(is_upgrade("1.0.0", "1.0.1-rc.1"));
        assert!(is_upgrade("1.0.0-rc.1", "1.0.0"));
        assert!(!is_upgrade("1.0.0", "1.0.0-rc.1"));
    }

    #[test]
    fn extract_target_from_github_shorthand() {
        assert_eq!(
            extract_github_target("github:omdsh-dev/DSH-better-sidebar"),
            target("omdsh-dev/DSH-better-sidebar", None)
        );
        assert_eq!(
            extract_github_target("github:baihejiangnan/dsh-session-context-menu#release/next"),
            target(
                "baihejiangnan/dsh-session-context-menu",
                Some("release/next")
            )
        );
        assert_eq!(
            extract_github_target("github:Small-tailqwq/dsh-deep-whale#path:/maid-atelier"),
            target("Small-tailqwq/dsh-deep-whale", None)
        );
        assert_eq!(
            extract_github_target(
                "github:RexSkz/test-git-subdir-fetch#beta&path:/packages/simple-react-app"
            ),
            target("RexSkz/test-git-subdir-fetch", Some("beta"))
        );
        assert_eq!(
            extract_github_target(
                "github:RexSkz/test-git-subdir-fetch#semver:^2.0.0&path:/packages/app"
            ),
            unsupported_target("RexSkz/test-git-subdir-fetch")
        );
    }

    #[test]
    fn extract_target_from_git_urls_and_codeload() {
        assert_eq!(
            extract_github_target(
                "git+https://github.com/omdsh-dev/DSH-better-sidebar.git#v0.16.1"
            ),
            target("omdsh-dev/DSH-better-sidebar", Some("v0.16.1"))
        );
        assert_eq!(
            extract_github_target("git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#next"),
            target("omdsh-dev/DSH-better-sidebar", Some("next"))
        );
        assert_eq!(
            extract_github_target("git+ssh://git@github.com:owner/repo.git#v1.2.3"),
            target("owner/repo", Some("v1.2.3"))
        );
        assert_eq!(
            extract_github_target("git+ssh://git@github.com:22/owner/repo.git#next"),
            target("owner/repo", Some("next"))
        );
        assert_eq!(
            extract_github_target("git+ssh://git@github.com:22:owner/repo.git#next"),
            target("owner/repo", Some("next"))
        );
        assert_eq!(
            extract_github_target("git@github.com:owner/repo.git#release/next"),
            target("owner/repo", Some("release/next"))
        );
        assert_eq!(
            extract_github_target("git+ssh://git@GITHUB.COM/Owner/Repo.git#next"),
            target("Owner/Repo", Some("next"))
        );
        assert_eq!(
            extract_github_target("https://codeload.github.com/omdsh-dev/DSH-better-sidebar/tar.gz/7dbd9b75e2fd65758d4e55f750319399b91255a2"),
            target("omdsh-dev/DSH-better-sidebar", None)
        );
    }

    #[test]
    fn encoded_fragment_is_decoded_once_before_selector_parsing() {
        assert_eq!(
            extract_github_target("github:owner/repo#release%2Fnext"),
            target("owner/repo", Some("release/next"))
        );
        assert_eq!(
            extract_github_target("github:owner/repo#path%3A%2Fpackages%2Fplugin"),
            target("owner/repo", None)
        );
        assert_eq!(
            extract_github_target("github:owner/repo#beta%26path%3A%2Fpackages%2Fplugin"),
            target("owner/repo", Some("beta"))
        );
        assert_eq!(
            extract_github_target("github:owner/repo#release%252Fnext"),
            target("owner/repo", Some("release%2Fnext"))
        );
        assert_eq!(
            extract_github_target("git+https://github.com/owner/repo.git#release%2Fnext"),
            target("owner/repo", Some("release/next"))
        );
        assert_eq!(
            extract_github_target("git+https://github.com/owner/repo.git#release%252Fnext"),
            target("owner/repo", Some("release%2Fnext"))
        );
    }

    #[test]
    fn unsupported_selector_fails_closed_without_a_commit_url() {
        let semver = extract_github_target("github:owner/repo#semver:%5E2.0.0").unwrap();
        let mixed = extract_github_target("github:owner/repo#next&semver:%5E2.0.0").unwrap();
        let malformed = extract_github_target("github:owner/repo#release%ZZnext").unwrap();
        let empty_selector = extract_github_target("github:owner/repo#next&").unwrap();
        assert_eq!(semver.reference, GitReference::Unsupported);
        assert_eq!(mixed.reference, GitReference::Unsupported);
        assert_eq!(malformed.reference, GitReference::Unsupported);
        assert_eq!(empty_selector.reference, GitReference::Unsupported);
        assert_eq!(github_commit_url(&semver), None);
        assert_eq!(github_commit_url(&mixed), None);
        assert_eq!(github_commit_url(&malformed), None);
        assert_eq!(github_commit_url(&empty_selector), None);
    }

    #[test]
    fn last_plain_reference_selector_wins() {
        assert_eq!(
            extract_github_target("github:owner/repo#old&path:/plugin&new"),
            target("owner/repo", Some("new"))
        );
    }

    #[test]
    fn codeload_archive_commit_is_current_state_not_update_target() {
        assert_eq!(
            extract_github_target("https://codeload.github.com/owner/repo/tar.gz/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            target("owner/repo", None)
        );
        assert_eq!(
            github_commit_url(&target("owner/repo", None).unwrap()).map(|url| url.to_string()),
            Some("https://api.github.com/repos/owner/repo/commits/HEAD".into())
        );
    }

    #[test]
    fn extract_target_for_plain_npm_or_incomplete_codeload_is_none() {
        assert_eq!(extract_github_target("dshmarket"), None);
        assert_eq!(extract_github_target("link:../plugin"), None);
        assert_eq!(extract_github_target("file:./local"), None);
        assert_eq!(extract_github_target("npm:dshmarket@^1.0"), None);
        assert_eq!(
            extract_github_target("https://codeload.github.com/owner/repo/tar.gz/"),
            None
        );
        assert_eq!(
            extract_github_target("https://notgithub.com/github.com/owner/repo.git#next"),
            None
        );
        assert_eq!(
            extract_github_target("https://example.com/path/github.com/owner/repo.git#next"),
            None
        );
        assert_eq!(
            extract_github_target(
                "https://evil.example/codeload.github.com/owner/repo/tar.gz/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            None
        );
        assert_eq!(
            extract_github_target(
                "https://codeload.github.com.evil.example/owner/repo/tar.gz/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            None
        );
        assert_eq!(
            extract_github_target(
                "ftp://codeload.github.com/owner/repo/tar.gz/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            None
        );
        assert_eq!(
            extract_github_target("ftp://github.com/owner/repo.git#next"),
            None
        );
    }

    #[test]
    fn commit_url_encodes_branch_slash_as_one_reference() {
        let target = GitHubTarget {
            repo: "owner/repo".into(),
            reference: GitReference::Named("release/next".into()),
        };
        assert_eq!(
            github_commit_url(&target).map(|url| url.to_string()),
            Some("https://api.github.com/repos/owner/repo/commits/release%2Fnext".into())
        );
    }

    #[test]
    fn exact_commit_is_immutable_without_network_resolution() {
        let sha = "7DBD9B75E2FD65758D4E55F750319399B91255A2";
        assert_eq!(
            immutable_commit(sha),
            Some("7dbd9b75e2fd65758d4e55f750319399b91255a2".into())
        );
        assert_eq!(immutable_commit("7dbd9b7"), None);
        assert_eq!(immutable_commit("not-a-commit"), None);
    }

    #[test]
    fn lock_commits_parsed_from_codeload_urls() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      dsh-better-sidebar:\n        specifier: github:omdsh-dev/DSH-better-sidebar\n        version: https://codeload.github.com/omdsh-dev/DSH-better-sidebar/tar.gz/7DBD9B75E2FD65758D4E55F750319399B91255A2\n";
        let dir = std::env::temp_dir().join(format!("dsh-updates-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([(
            "dsh-better-sidebar".into(),
            "github:omdsh-dev/DSH-better-sidebar".into(),
        )]);
        let commits = read_locked_commits(&dir, &specs);
        assert_eq!(
            commits.get("dsh-better-sidebar"),
            Some(&"7dbd9b75e2fd65758d4e55f750319399b91255a2".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lock_commits_keep_distinct_direct_dependencies_from_same_repo() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      stable-plugin:\n        specifier: github:owner/repo#stable\n        version: https://codeload.github.com/owner/repo/tar.gz/1111111\n      canary-plugin:\n        specifier: github:owner/repo#canary\n        version: https://codeload.github.com/owner/repo/tar.gz/2222222\npackages:\n  transitive@https://codeload.github.com/owner/repo/tar.gz/3333333:\n    resolution: {tarball: https://codeload.github.com/owner/repo/tar.gz/3333333}\n";
        let dir =
            std::env::temp_dir().join(format!("dsh-updates-lock-duplicate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([
            ("stable-plugin".into(), "github:owner/repo#stable".into()),
            ("canary-plugin".into(), "github:owner/repo#canary".into()),
        ]);

        let commits = read_locked_commits(&dir, &specs);

        assert_eq!(commits.get("stable-plugin"), Some(&"1111111".to_string()));
        assert_eq!(commits.get("canary-plugin"), Some(&"2222222".to_string()));
        assert_eq!(commits.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_key_changes_when_direct_lockfile_commit_changes() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-updates-lock-transition-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pnpm-lock.yaml");
        let prefix = "importers:\n  .:\n    configDependencies: {}\n    packageManagerDependencies:\n      pnpm:\n        specifier: 12.0.0\n        version: 12.0.0\n---\nimporters:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/owner/repo/tar.gz/";
        let specs = HashMap::from([("plugin".into(), "github:owner/repo".into())]);

        std::fs::write(&path, format!("{prefix}1111111\n")).unwrap();
        let before = read_locked_commits(&dir, &specs);
        let before_key = cache_key("plugin", "github:owner/repo", "1.0.0", &before, "");

        std::fs::write(&path, format!("{prefix}2222222\n")).unwrap();
        let after = read_locked_commits(&dir, &specs);
        let after_key = cache_key("plugin", "github:owner/repo", "1.0.0", &after, "");

        assert_eq!(before.get("plugin"), Some(&"1111111".to_string()));
        assert_eq!(after.get("plugin"), Some(&"2222222".to_string()));
        assert_ne!(before_key, after_key);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_multidocument_lockfile_fails_closed() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/owner/repo/tar.gz/1111111\n---\nmalformed: [\n";
        let dir =
            std::env::temp_dir().join(format!("dsh-updates-lock-malformed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([("plugin".into(), "github:owner/repo".into())]);

        assert!(read_locked_commits(&dir, &specs).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_current_importer_documents_fail_closed() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/owner/repo/tar.gz/1111111\n---\nimporters:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/owner/repo/tar.gz/2222222\n";
        let dir =
            std::env::temp_dir().join(format!("dsh-updates-lock-ambiguous-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([("plugin".into(), "github:owner/repo".into())]);

        assert!(read_locked_commits(&dir, &specs).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_current_importer_with_repo_mismatch_fails_closed() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/owner/repo/tar.gz/1111111\n---\nimporters:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:other/repo\n        version: https://codeload.github.com/other/repo/tar.gz/2222222\n";
        let dir = std::env::temp_dir().join(format!(
            "dsh-updates-lock-ambiguous-mismatch-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([("plugin".into(), "github:owner/repo".into())]);

        assert!(read_locked_commits(&dir, &specs).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lock_commit_repo_must_match_direct_spec() {
        let lock = "\
importers:\n  .:\n    dependencies:\n      plugin:\n        specifier: github:owner/repo\n        version: https://codeload.github.com/other/repo/tar.gz/1111111\n";
        let dir =
            std::env::temp_dir().join(format!("dsh-updates-lock-mismatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        let specs = HashMap::from([("plugin".into(), "github:owner/repo".into())]);

        assert!(read_locked_commits(&dir, &specs).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_key_changes_with_version() {
        let locked = HashMap::new();
        let a = cache_key("p", "spec", "1.0.0", &locked, "");
        let b = cache_key("p", "spec", "1.0.1", &locked, "");
        assert_ne!(a, b);
        let c = cache_key("p", "spec2", "1.0.0", &locked, "");
        assert_ne!(a, c);
    }

    #[test]
    fn cache_key_changes_with_locked_git_commit() {
        let missing = HashMap::new();
        let missing_key = cache_key("p", "github:Owner/Repo", "1.0.0", &missing, "");
        let mut locked = HashMap::from([("p".into(), "aaaaaaa".into())]);
        let a = cache_key("p", "github:Owner/Repo", "1.0.0", &locked, "");
        let cache = HashMap::from([(a.clone(), true)]);

        locked.insert("p".into(), "bbbbbbb".into());
        let b = cache_key("p", "github:Owner/Repo", "1.0.0", &locked, "");

        assert_ne!(missing_key, a);
        assert_ne!(a, b);
        assert!(!cache.contains_key(&b));
    }

    #[test]
    fn registry_cache_key_ignores_unrelated_git_commits() {
        let mut locked = HashMap::from([("other-plugin".into(), "aaaaaaa".into())]);
        let a = cache_key("p", "^1.0.0", "1.0.0", &locked, "");

        locked.insert("other-plugin".into(), "bbbbbbb".into());
        let b = cache_key("p", "^1.0.0", "1.0.0", &locked, "");

        assert_eq!(a, b);
    }

    #[test]
    fn registry_name_encoded() {
        assert_eq!(encode_registry_name("@scope/name"), "%40scope%2Fname");
        assert_eq!(encode_registry_name("dshmarket"), "dshmarket");
    }
}
