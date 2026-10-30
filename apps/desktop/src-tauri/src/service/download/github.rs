use crate::AppHandle;
use crate::config;

const DSH_PKG_GITHUB_API: &str = "https://api.github.com/repos/dsh-tauri-desk/deepseek-harness-pkg";
const DSH_PKG_REPO: &str = "https://github.com/dsh-tauri-desk/deepseek-harness-pkg";
const GITHUB_RELEASES_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, serde::Serialize)]
pub struct LatestDshPkg {
    pub tag: String,
    pub commit: String,
    pub asset_url: String,
    pub digest: Option<String>,
}

fn github_client(app_handle: &AppHandle) -> Result<reqwest::Client, String> {
    config::proxy::http_client_builder(app_handle)?
        .user_agent("deepseek-harness-desktop")
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

async fn github_api_get(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    if crate::service::download::github_api::rate_limited() {
        return Err("GitHub API rate limited (cached, using fallback sources)".to_string());
    }
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if res.status() == reqwest::StatusCode::FORBIDDEN {
        crate::service::download::github_api::mark_rate_limited();
        return Err(format!("GitHub API rate limited: HTTP {}", res.status()));
    }
    res.error_for_status().map_err(|e| e.to_string())
}

async fn fetch_releases_latest(client: &reqwest::Client) -> Result<serde_json::Value, String> {
    github_api_get(client, &format!("{DSH_PKG_GITHUB_API}/releases/latest"))
        .await
        .map_err(|e| format!("Latest release request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse latest release response: {e}"))
}

fn parse_release_list_from_html(body: &str) -> Vec<DshPkgReleaseMeta> {
    let mut releases = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let marker = "releases/tag/";
    let mut cursor = 0;
    while let Some(relative) = body[cursor..].find(marker) {
        let start = cursor + relative + marker.len();
        let Some(end) = body[start..].find(['"', '\'', '?']) else {
            break;
        };
        let tag = &body[start..start + end];
        let next = body[start + end..]
            .find(marker)
            .map(|offset| start + end + offset)
            .unwrap_or(body.len());
        let entry = &body[start..next];
        if !tag.is_empty() && seen.insert(tag.to_string()) {
            releases.push(DshPkgReleaseMeta {
                tag: tag.to_string(),
                prerelease: entry.contains("Pre-release"),
            });
        }
        cursor = next;
    }
    releases
}

async fn fetch_tag_commit(client: &reqwest::Client, tag: &str) -> Result<String, String> {
    let commit: serde_json::Value =
        github_api_get(client, &format!("{DSH_PKG_GITHUB_API}/commits/{tag}"))
            .await
            .map_err(|e| format!("Release commit request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse release commit response: {e}"))?;
    commit
        .get("sha")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing sha in release commit response".to_string())
}

fn commit_fallback_from_tag(tag: &str) -> String {
    tag.rsplit('-').next().unwrap_or(tag).to_string()
}

fn first_non_preview_tag_from_atom(body: &str) -> Option<String> {
    let mut rest = body;
    while let Some(start) = rest.find("<entry>") {
        let end = rest[start..]
            .find("</entry>")
            .map(|e| start + e)
            .unwrap_or(rest.len());
        let entry = &rest[start..end];
        let tag = entry
            .split("releases/tag/")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .filter(|t| !t.is_empty());
        if let Some(tag) = tag {
            if is_preview_tag(tag) {
                log::debug!("DSH_ATOM: skip preview release {tag}");
            } else {
                return Some(tag.to_string());
            }
        }
        rest = &rest[end..];
    }
    None
}

async fn fetch_latest_dsh_tag_from_atom(app_handle: &AppHandle) -> Result<String, String> {
    let client = github_client(app_handle)?;
    let body = client
        .get(format!("{DSH_PKG_REPO}/releases.atom"))
        .send()
        .await
        .map_err(|e| format!("DSH_ATOM: {e}"))?
        .error_for_status()
        .map_err(|e| format!("DSH_ATOM: {e}"))?
        .text()
        .await
        .map_err(|e| format!("DSH_ATOM: {e}"))?;
    first_non_preview_tag_from_atom(&body)
        .ok_or_else(|| "DSH_ATOM: no non-preview release in atom feed".to_string())
}

fn parse_digest_from_expanded_assets(body: &str, expected_name: &str) -> Option<String> {
    let pos = body.find(expected_name)?;
    let mut end = (pos + 4096).min(body.len());
    while end > pos && !body.is_char_boundary(end) {
        end -= 1;
    }
    let window = &body[pos..end];
    const START: &str = "sha256:";
    let hash_start = window.find(START)?;
    let hash = &window[hash_start + START.len()..];
    let hex_end = hash
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(hash.len());
    if hex_end != 64 {
        return None;
    }
    Some(format!("sha256:{}", &hash[..64]))
}

async fn fetch_dsh_digest_from_expanded_assets(
    client: &reqwest::Client,
    tag: &str,
    expected_name: &str,
) -> Result<Option<String>, String> {
    let body = client
        .get(format!("{DSH_PKG_REPO}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .map_err(|e| format!("DSH_EXPANDED: {e}"))?
        .error_for_status()
        .map_err(|e| format!("DSH_EXPANDED: {e}"))?
        .text()
        .await
        .map_err(|e| format!("DSH_EXPANDED: {e}"))?;
    Ok(parse_digest_from_expanded_assets(&body, expected_name))
}

/// 回退到最新一条非预览版 release 供更新/安装判定（issue #299：最新 alpha 发布后
pub async fn fetch_latest_dsh_pkg_info(app_handle: &AppHandle) -> Result<LatestDshPkg, String> {
    let client = github_client(app_handle)?;
    let expected_name = config::get_dsh_download_url()?
        .rsplit('/')
        .next()
        .ok_or_else(|| "Missing DSH asset filename".to_string())?
        .to_string();

    let api_release = match fetch_releases_latest(&client).await {
        Ok(release) => Some(release),
        Err(e) => {
            if crate::service::download::github_api::rate_limited() {
                log::debug!(
                    "GitHub API latest release unavailable (rate-limited), using fallback sources: {e}"
                );
            } else {
                log::warn!(
                    "GitHub API latest release unavailable ({}), falling back to atom feed",
                    e
                );
            }
            None
        }
    };

    let tag_name = match &api_release {
        Some(release) => release
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing tag_name in latest release response".to_string())?
            .to_string(),
        None => fetch_latest_dsh_tag_from_atom(app_handle).await?,
    };

    //     让初始化/更新流程整体卡死（issue #299），改为回退到最新非预览版 release，
    if is_preview_tag(&tag_name) {
        log::info!(
            "DSH_SKIP_PREVIEW: latest release {} is a preview, falling back to latest non-preview release",
            tag_name
        );
        return fetch_latest_non_preview(app_handle).await;
    }

    let commit = match fetch_tag_commit(&client, &tag_name).await {
        Ok(sha) => sha,
        Err(e) => {
            if crate::service::download::github_api::rate_limited() {
                log::debug!("GitHub API commit resolution rate-limited, using build-id fallback");
            } else {
                log::warn!(
                    "Failed to resolve commit for tag {} ({}), using build-id fallback",
                    tag_name,
                    e
                );
            }
            commit_fallback_from_tag(&tag_name)
        }
    };

    let (asset_url, mut digest) = match api_release.as_ref() {
        Some(release) => {
            let asset = release
                .get("assets")
                .and_then(|value| value.as_array())
                .and_then(|assets| {
                    assets.iter().find(|asset| {
                        asset.get("name").and_then(|value| value.as_str())
                            == Some(expected_name.as_str())
                    })
                });
            let asset_url = asset
                .and_then(|a| a.get("browser_download_url").and_then(|v| v.as_str()))
                .map(|u| u.to_string())
                .unwrap_or_else(|| config::get_dsh_download_url().unwrap_or_default());
            let digest = asset
                .and_then(|a| a.get("digest").and_then(|v| v.as_str()))
                .filter(|v| v.starts_with("sha256:"))
                .map(|v| v.to_string());
            (asset_url, digest)
        }
        None => (config::get_dsh_download_url_for_tag(&tag_name)?, None),
    };

    if digest.is_none() {
        match fetch_dsh_digest_from_expanded_assets(&client, &tag_name, &expected_name).await {
            Ok(Some(d)) => {
                log::info!(
                    "Trusted digest unavailable from GitHub API, recovered from release HTML for {}",
                    expected_name
                );
                digest = Some(d);
            }
            Ok(None) => {
                log::warn!(
                    "No digest found in release HTML for {} (tag {})",
                    expected_name,
                    tag_name
                );
            }
            Err(e) => {
                log::warn!("Failed to fetch digest from release HTML: {}", e);
            }
        }
    }

    Ok(LatestDshPkg {
        tag: tag_name,
        commit,
        asset_url,
        digest,
    })
}

pub async fn fetch_dsh_pkg_version(
    app_handle: &AppHandle,
    version: &str,
) -> Result<LatestDshPkg, String> {
    let release = fetch_dsh_pkg_releases(app_handle)
        .await?
        .into_iter()
        .find(|release| parse_version_from_tag(&release.tag).as_deref() == Some(version))
        .ok_or_else(|| format!("DSH_RECOMMENDED_NOT_FOUND: no release found for {version}"))?;
    fetch_dsh_pkg_asset(app_handle, &release.tag).await
}

async fn fetch_latest_non_preview(app_handle: &AppHandle) -> Result<LatestDshPkg, String> {
    let tag = fetch_dsh_pkg_releases(app_handle)
        .await?
        .into_iter()
        .find(|m| !m.prerelease && !is_preview_tag(&m.tag))
        .map(|m| m.tag)
        .ok_or_else(|| {
            "DSH_PREVIEW_RELEASE: no non-preview release available, not an update".to_string()
        })?;
    fetch_dsh_pkg_asset(app_handle, &tag).await
}

pub async fn fetch_dsh_pkg_asset(
    app_handle: &AppHandle,
    tag: &str,
) -> Result<LatestDshPkg, String> {
    let client = github_client(app_handle)?;
    let expected_name = config::get_dsh_download_url()?
        .rsplit('/')
        .next()
        .ok_or_else(|| "Missing DSH asset filename".to_string())?
        .to_string();

    let json: Option<serde_json::Value> = match github_api_get(
        &client,
        &format!("{DSH_PKG_GITHUB_API}/releases/tags/{tag}"),
    )
    .await
    {
        Ok(release) => match release.json().await {
            Ok(json) => Some(json),
            Err(e) => {
                log::warn!("Failed to parse release {tag} response: {e}");
                None
            }
        },
        Err(e) => {
            log::warn!("Release {tag} request failed ({e}), using deterministic asset URL");
            None
        }
    };

    let asset = json
        .as_ref()
        .and_then(|json| json.get("assets"))
        .and_then(|value| value.as_array())
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset.get("name").and_then(|value| value.as_str()) == Some(expected_name.as_str())
            })
        });
    let asset_url = asset
        .and_then(|a| a.get("browser_download_url").and_then(|v| v.as_str()))
        .map(|u| u.to_string())
        .unwrap_or_else(|| config::get_dsh_download_url_for_tag(tag).unwrap_or_default());
    let mut digest = asset
        .and_then(|a| a.get("digest").and_then(|v| v.as_str()))
        .filter(|v| v.starts_with("sha256:"))
        .map(|v| v.to_string());

    if digest.is_none() {
        match fetch_dsh_digest_from_expanded_assets(&client, tag, &expected_name).await {
            Ok(Some(d)) => {
                log::info!(
                    "Trusted digest unavailable from GitHub API, recovered from release HTML for {}",
                    expected_name
                );
                digest = Some(d);
            }
            Ok(None) => {
                log::warn!(
                    "No digest found in release HTML for {} (tag {})",
                    expected_name,
                    tag
                );
            }
            Err(e) => {
                log::warn!("Failed to fetch digest from release HTML: {}", e);
            }
        }
    }

    Ok(LatestDshPkg {
        tag: tag.to_string(),
        commit: commit_fallback_from_tag(tag),
        asset_url,
        digest,
    })
}

pub fn parse_version_from_tag(tag: &str) -> Option<String> {
    let has_dsh_prefix = tag.starts_with("dsh-");
    let tag = tag.strip_prefix("dsh-").unwrap_or(tag);
    if let Some(version) = tag.strip_prefix("src-") {
        let version = if has_dsh_prefix {
            version.rsplit_once('-').map(|(version, _)| version)?
        } else {
            version
        };
        return semver::Version::parse(version)
            .ok()
            .map(|_| version.to_string());
    }
    let version = has_dsh_prefix.then(|| tag.rsplit_once('-').map(|(version, _)| version))??;
    (!version.is_empty()).then(|| version.to_string())
}

pub fn is_preview_tag(tag: &str) -> bool {
    let Some(version) = parse_version_from_tag(tag) else {
        return false;
    };
    let Ok(parsed) = semver::Version::parse(&version) else {
        return false;
    };
    const PREVIEW_MARKERS: [&str; 5] = ["preview", "beta", "alpha", "canary", "next"];
    parsed
        .pre
        .as_str()
        .split('.')
        .any(|id| PREVIEW_MARKERS.iter().any(|m| id.starts_with(m)))
}

/// （issue #92）。因此需归一化：记录 tag 与最新 tag 相同（同一次发布，任何
pub fn record_matches_latest_release(
    record_commit: Option<&str>,
    record_tag: Option<&str>,
    latest: &LatestDshPkg,
) -> bool {
    record_tag == Some(latest.tag.as_str())
        || record_commit.is_some_and(|rc| {
            rc == latest.commit.as_str() || rc == commit_fallback_from_tag(&latest.tag)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate,
    HealUpToDate,
    UpdateAvailable,
}

pub fn resolve_update(
    record_commit: Option<&str>,
    record_tag: Option<&str>,
    installed_version: Option<&str>,
    latest: &LatestDshPkg,
    legacy_tags: &[(String, String)],
) -> UpdateCheck {
    let (Some(installed), Some(latest_version)) =
        (installed_version, parse_version_from_tag(&latest.tag))
    else {
        return if record_matches_latest_release(record_commit, record_tag, latest) {
            UpdateCheck::UpToDate
        } else {
            UpdateCheck::UpdateAvailable
        };
    };

    let same_version = match (
        semver::Version::parse(installed).ok(),
        semver::Version::parse(&latest_version).ok(),
    ) {
        (Some(installed), Some(latest)) => installed == latest,
        _ => installed == latest_version,
    };
    if !same_version {
        return UpdateCheck::UpdateAvailable;
    }
    if record_matches_latest_release(record_commit, record_tag, latest) {
        return UpdateCheck::UpToDate;
    }
    let record_behind_latest = |record_version: &str| -> bool {
        match (
            semver::Version::parse(record_version).ok(),
            semver::Version::parse(&latest_version).ok(),
        ) {
            (Some(record), Some(latest)) => record < latest,
            _ => record_version < latest_version.as_str(),
        }
    };
    match record_tag.and_then(parse_version_from_tag) {
        Some(record_version) if record_behind_latest(&record_version) => UpdateCheck::HealUpToDate,
        Some(_) => UpdateCheck::UpdateAvailable,
        None => {
            let matches: Vec<&(String, String)> = legacy_tags
                .iter()
                .filter(|(_, commit)| Some(commit.as_str()) == record_commit)
                .collect();
            match matches.as_slice() {
                [(tag, _)] if tag.as_str() == latest.tag.as_str() => UpdateCheck::UpToDate,
                [(tag, _)] => match parse_version_from_tag(tag) {
                    Some(record_version) if record_behind_latest(&record_version) => {
                        UpdateCheck::HealUpToDate
                    }
                    _ => UpdateCheck::UpdateAvailable,
                },
                _ if matches.len() > 1 => UpdateCheck::UpdateAvailable,
                _ => UpdateCheck::HealUpToDate,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DshPkgReleaseMeta {
    pub tag: String,
    pub prerelease: bool,
}

async fn fetch_dsh_pkg_releases_from_html(
    client: &reqwest::Client,
) -> Result<Vec<DshPkgReleaseMeta>, String> {
    let body = client
        .get(format!("{DSH_PKG_REPO}/releases"))
        .send()
        .await
        .map_err(|e| format!("Release page request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Release page request failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read release page: {e}"))?;
    let releases = parse_release_list_from_html(&body);
    if releases.is_empty() {
        return Err("Release page contained no release tags".to_string());
    }
    Ok(releases)
}

pub async fn fetch_dsh_pkg_releases(
    app_handle: &AppHandle,
) -> Result<Vec<DshPkgReleaseMeta>, String> {
    let client = github_client(app_handle)?;
    let mut all_releases = Vec::new();
    let mut page = 1;
    loop {
        let response = match github_api_get(
            &client,
            &format!(
                "{DSH_PKG_GITHUB_API}/releases?per_page={GITHUB_RELEASES_PAGE_SIZE}&page={page}"
            ),
        )
        .await
        {
            Ok(response) => response,
            Err(api_error) => {
                if page > 1 {
                    return Err(format!(
                        "DSH_RELEASE_LIST_INCOMPLETE: page {page} request failed before pagination completed: {api_error}"
                    ));
                }
                log::warn!(
                    "GitHub API release list unavailable ({}), falling back to Releases page",
                    api_error
                );
                return fetch_dsh_pkg_releases_from_html(&client)
                    .await
                    .map_err(|page_error| {
                        format!("Release list request failed: {api_error}; {page_error}")
                    });
            }
        };
        let releases: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse release list response: {e}"))?;
        let entries = releases
            .as_array()
            .ok_or_else(|| "DSH_RELEASE_LIST_INVALID: response was not an array".to_string())?;
        let last_page = entries.len() < GITHUB_RELEASES_PAGE_SIZE;
        let page_releases = entries
            .iter()
            .filter_map(|entry| {
                let tag = entry.get("tag_name")?.as_str()?.to_string();
                let prerelease = entry
                    .get("prerelease")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(DshPkgReleaseMeta { tag, prerelease })
            })
            .collect::<Vec<_>>();
        all_releases.extend(page_releases);
        if last_page {
            return Ok(all_releases);
        }
        page += 1;
    }
}

pub async fn fetch_dsh_pkg_tags(app_handle: &AppHandle) -> Result<Vec<(String, String)>, String> {
    let client = github_client(app_handle)?;
    let tags: serde_json::Value = github_api_get(
        &client,
        &format!("{}/tags?per_page=100", DSH_PKG_GITHUB_API),
    )
    .await
    .map_err(|e| format!("Release tags request failed: {e}"))?
    .json()
    .await
    .map_err(|e| format!("Failed to parse release tags response: {e}"))?;

    Ok(tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let sha = entry.get("commit")?.get("sha")?.as_str()?.to_string();
            Some((name, sha))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latest(tag: &str, commit: &str) -> LatestDshPkg {
        LatestDshPkg {
            tag: tag.to_string(),
            commit: commit.to_string(),
            asset_url: "https://example.invalid/dsh.zip".to_string(),
            digest: Some(format!("sha256:{}", "0".repeat(64))),
        }
    }

    #[test]
    fn atom_fallback_download_url_is_pinned_to_resolved_tag() {
        let tag = "dsh-src-0.1.2-alpha.1-33260039971";
        let url = config::get_dsh_download_url_for_tag(tag).expect("dsh url");
        assert!(url.contains(&format!("/releases/download/{tag}/")));
        assert!(!url.contains("/releases/latest/download/"));
        assert!(
            url.ends_with("deepseek-harness-pkg-windows.asar.zip")
                || url.ends_with("deepseek-harness-pkg-linux.asar.zip")
                || url.ends_with("deepseek-harness-pkg-linux-arm64.asar.zip")
                || url.ends_with("deepseek-harness-pkg-macos-arm64.asar.zip")
                || url.ends_with("deepseek-harness-pkg-macos-x64.asar.zip")
        );
    }

    #[test]
    fn commit_fallback_extracts_build_id_from_tag() {
        assert_eq!(
            commit_fallback_from_tag("dsh-0.1.0-rc.8-32331963388"),
            "32331963388"
        );
        assert_eq!(commit_fallback_from_tag("dsh-0.2.0"), "0.2.0");
    }

    #[test]
    fn parses_digest_from_expanded_assets_html() {
        let html = "…/deepseek-harness-pkg-windows.zip…<span>sha256:4d5416766eb4a66e81b83532abeea64de7e7e2e0bac69a4f0c0508e1d91936c0</span>";
        let got = parse_digest_from_expanded_assets(html, "deepseek-harness-pkg-windows.zip");
        assert_eq!(
            got.as_deref(),
            Some("sha256:4d5416766eb4a66e81b83532abeea64de7e7e2e0bac69a4f0c0508e1d91936c0")
        );
        assert_eq!(
            parse_digest_from_expanded_assets(html, "deepseek-harness-pkg-linux.zip"),
            None
        );
        assert_eq!(
            parse_digest_from_expanded_assets(
                "<p>no digest here</p>",
                "deepseek-harness-pkg-windows.zip"
            ),
            None
        );
    }

    #[test]
    fn parsed_digest_window_clamps_to_char_boundary() {
        let name = "deepseek-harness-pkg-windows.zip";
        let digest = "4d5416766eb4a66e81b83532abeea64de7e7e2e0bac69a4f0c0508e1d91936c0";
        let prefix = format!("{name}<span>sha256:{digest}</span>");
        let mut body = prefix.clone();
        let needed = 4094usize.saturating_sub(body.len());
        body.push_str(&"a".repeat(needed));
        body.push('中');
        body.push_str(&"a".repeat(16));
        assert!(body.len() > 4096);
        let got = parse_digest_from_expanded_assets(&body, name);
        let expected = format!("sha256:{digest}");
        assert_eq!(got.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn parse_version_from_tag_formats() {
        assert_eq!(
            parse_version_from_tag("dsh-0.1.0-rc.7-32054485373").as_deref(),
            Some("0.1.0-rc.7")
        );
        assert_eq!(
            parse_version_from_tag("dsh-0.1.0-rc.6-31773193667").as_deref(),
            Some("0.1.0-rc.6")
        );
        assert_eq!(parse_version_from_tag("dsh-0.2.0"), None);
        assert_eq!(parse_version_from_tag("0.1.0-rc.7-abc"), None);
        assert_eq!(parse_version_from_tag(""), None);
        assert_eq!(
            parse_version_from_tag("src-0.1.2-alpha.1").as_deref(),
            Some("0.1.2-alpha.1")
        );
        assert_eq!(
            parse_version_from_tag("dsh-src-0.1.2-alpha.1-33260039971").as_deref(),
            Some("0.1.2-alpha.1")
        );
    }

    #[test]
    fn preview_tag_detection_by_version_marker() {
        assert!(is_preview_tag("dsh-0.2.0-preview.1-32490000001"));
        assert!(is_preview_tag("dsh-0.2.0-beta.1-32490000002"));
        assert!(is_preview_tag("dsh-0.2.0-alpha.2-32490000003"));
        assert!(is_preview_tag("dsh-0.2.0-canary.1-32490000004"));
        assert!(is_preview_tag("dsh-0.2.0-next.3-32490000005"));
        assert!(!is_preview_tag("dsh-0.1.1-rc.2-32485170079"));
        assert!(!is_preview_tag("dsh-0.1.0-rc.8-32342588166"));
        assert!(!is_preview_tag("dsh-0.2.0-32490000006"));
        assert!(!is_preview_tag("dsh-0.2.0"));
        assert!(!is_preview_tag(""));
    }

    #[test]
    fn release_page_parser_preserves_preview_labels_and_deduplicates_links() {
        let html = concat!(
            r#"<a href="/dsh-tauri-desk/deepseek-harness-pkg/releases/tag/dsh-0.2.0-beta.1-1">Pre-release</a>"#,
            r#"<a href="/dsh-tauri-desk/deepseek-harness-pkg/releases/tag/dsh-0.2.0-beta.1-1">same release</a>"#,
            r#"<a href="/dsh-tauri-desk/deepseek-harness-pkg/releases/tag/dsh-0.1.1-rc.2-2">Release</a>"#,
        );
        let releases = parse_release_list_from_html(html);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].tag, "dsh-0.2.0-beta.1-1");
        assert!(releases[0].prerelease);
        assert_eq!(releases[1].tag, "dsh-0.1.1-rc.2-2");
        assert!(!releases[1].prerelease);
    }

    /// 最新 release 是预览版时，应回退到最新一条「非预览」release（issue #299）。
    #[test]
    fn non_preview_selection_skips_preview_tags_and_labels() {
        let pick_non_preview = |releases: &[DshPkgReleaseMeta]| {
            releases
                .iter()
                .find(|m| !m.prerelease && !is_preview_tag(&m.tag))
                .map(|m| m.tag.clone())
        };

        // 最新是标签预览版（dsh-…-alpha.3，issue #299 现场：dsh-0.1.2-alpha.3）
        let releases = vec![
            DshPkgReleaseMeta {
                tag: "dsh-0.1.2-alpha.3-33444825807".to_string(),
                prerelease: false,
            },
            DshPkgReleaseMeta {
                tag: "dsh-0.1.1-rc.2-32485170079".to_string(),
                prerelease: true,
            },
            DshPkgReleaseMeta {
                tag: "dsh-0.1.1-rc.8-32342588166".to_string(),
                prerelease: false,
            },
        ];
        assert_eq!(
            pick_non_preview(&releases).as_deref(),
            Some("dsh-0.1.1-rc.8-32342588166")
        );

        let all_preview = vec![
            DshPkgReleaseMeta {
                tag: "dsh-0.2.0-preview.1-32490000001".to_string(),
                prerelease: false,
            },
            DshPkgReleaseMeta {
                tag: "dsh-0.1.0-rc.7-32054485373".to_string(),
                prerelease: true,
            },
        ];
        assert_eq!(pick_non_preview(&all_preview), None);
        assert_eq!(pick_non_preview(&[]), None);
    }

    #[test]
    fn atom_fallback_skips_preview_and_picks_next_non_preview() {
        let feed = concat!(
            "<feed>",
            "<entry><link rel=\"alternate\" href=\"/x/deepseek-harness-pkg/releases/tag/dsh-0.2.0-preview.1-32490000001\"/></entry>",
            "<entry><link rel=\"alternate\" href=\"/x/deepseek-harness-pkg/releases/tag/dsh-0.1.1-rc.2-32485170079\"/></entry>",
            "</feed>"
        );
        assert_eq!(
            first_non_preview_tag_from_atom(feed).as_deref(),
            Some("dsh-0.1.1-rc.2-32485170079")
        );
        let all_preview = concat!(
            "<feed>",
            "<entry><link rel=\"alternate\" href=\"/x/deepseek-harness-pkg/releases/tag/dsh-0.2.0-preview.1-32490000001\"/></entry>",
            "<entry><link rel=\"alternate\" href=\"/x/deepseek-harness-pkg/releases/tag/dsh-0.2.0-beta.1-32490000002\"/></entry>",
            "</feed>"
        );
        assert_eq!(first_non_preview_tag_from_atom(all_preview), None);
        assert_eq!(first_non_preview_tag_from_atom(""), None);
    }

    #[test]
    fn resolve_matching_commit_is_up_to_date() {
        let latest = latest(
            "dsh-0.1.0-rc.7-32054485373",
            "6c659bb2636b3ad396a204c4c6ff110276fa3a09",
        );
        let decision = resolve_update(
            Some("6c659bb2636b3ad396a204c4c6ff110276fa3a09"),
            Some("dsh-0.1.0-rc.7-32054485373"),
            Some("0.1.0-rc.7"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpToDate);
    }

    #[test]
    fn resolve_build_id_record_vs_real_sha_latest_is_up_to_date() {
        // issue #92 现场：限流期安装把 build-id 写进记录，API 恢复后的检查解析出
        let latest = latest(
            "dsh-0.1.1-rc.2-32485170079",
            "1eed6dd6c0780000000000000000000000000000",
        );
        let decision = resolve_update(
            Some("32485170079"), // 记录 commit：限流期写入的 build-id
            Some("dsh-0.1.1-rc.2-32485170079"),
            Some("0.1.1-rc.2"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpToDate);
    }

    #[test]
    fn resolve_real_sha_record_vs_build_id_latest_is_up_to_date() {
        let latest = latest(
            "dsh-0.1.1-rc.2-32485170079",
            "32485170079", // 本次检查限流，commit 兜底为 build-id
        );
        let decision = resolve_update(
            Some("1eed6dd6c0780000000000000000000000000000"), // 记录：完整 SHA
            Some("dsh-0.1.1-rc.2-32485170079"),
            Some("0.1.1-rc.2"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpToDate);
    }

    #[test]
    fn resolve_different_installed_version_is_update() {
        let latest = latest(
            "dsh-0.1.0-rc.7-32054485373",
            "6c659bb2636b3ad396a204c4c6ff110276fa3a09",
        );
        let decision = resolve_update(
            Some("564019027fd9469991aef6e57bb0a96325491c4e"),
            Some("dsh-0.1.0-rc.6-31773193667"),
            Some("0.1.0-rc.6"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }

    #[test]
    fn resolve_same_version_hotfix_is_update() {
        let latest = latest(
            "dsh-0.1.0-rc.6-31773193667",
            "564019027fd9469991aef6e57bb0a96325491c4e",
        );
        let decision = resolve_update(
            Some("995e261e117617780dc50db16c70d445255978fd"),
            Some("dsh-0.1.0-rc.6-31762761461"),
            Some("0.1.0-rc.6"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }

    #[test]
    fn resolve_stale_record_behind_files_heals() {
        let latest = latest(
            "dsh-0.1.0-rc.7-32054485373",
            "6c659bb2636b3ad396a204c4c6ff110276fa3a09",
        );
        let decision = resolve_update(
            Some("564019027fd9469991aef6e57bb0a96325491c4e"),
            Some("dsh-0.1.0-rc.6-31773193667"),
            Some("0.1.0-rc.7"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::HealUpToDate);
    }

    #[test]
    fn resolve_legacy_record_without_tag_heals_via_tags_lookup() {
        let latest = latest(
            "dsh-0.1.0-rc.7-32054485373",
            "6c659bb2636b3ad396a204c4c6ff110276fa3a09",
        );
        let tags = vec![(
            "dsh-0.1.0-rc.6-31773193667".to_string(),
            "564019027fd9469991aef6e57bb0a96325491c4e".to_string(),
        )];
        let decision = resolve_update(
            Some("564019027fd9469991aef6e57bb0a96325491c4e"),
            None,
            Some("0.1.0-rc.7"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::HealUpToDate);
    }

    #[test]
    fn resolve_legacy_same_version_still_updates() {
        let latest = latest(
            "dsh-0.1.0-rc.6-31773193667",
            "564019027fd9469991aef6e57bb0a96325491c4e",
        );
        let tags = vec![(
            "dsh-0.1.0-rc.6-31762761461".to_string(),
            "995e261e117617780dc50db16c70d445255978fd".to_string(),
        )];
        let decision = resolve_update(
            Some("995e261e117617780dc50db16c70d445255978fd"),
            None,
            Some("0.1.0-rc.6"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }

    #[test]
    fn resolve_without_version_metadata_falls_back_to_update() {
        let latest = latest("0.1.0-rc.7", "6c659bb2636b3ad396a204c4c6ff110276fa3a09");
        let decision = resolve_update(
            Some("564019027fd9469991aef6e57bb0a96325491c4e"),
            None,
            Some("0.1.0-rc.7"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }

    #[test]
    fn resolve_rc7_installed_rc8_released_is_update() {
        let latest = latest(
            "dsh-0.1.0-rc.8-32331963388",
            "6c659bb2636b3ad396a204c4c6ff110276fa3a09", // 与已装记录相同 commit
        );
        let decision = resolve_update(
            Some("6c659bb2636b3ad396a204c4c6ff110276fa3a09"), // rc.7 记录
            Some("dsh-0.1.0-rc.7-32054485373"),
            Some("0.1.0-rc.7"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }

    #[test]
    fn resolve_rc9_record_behind_rc10_heals() {
        let latest = latest(
            "dsh-0.1.0-rc.10-40000000000",
            "aabbccddeeff00112233445566778899aabbccdd",
        );
        let decision = resolve_update(
            Some("112233445566778899aabbccddeeff0011223344"),
            Some("dsh-0.1.0-rc.9-39000000000"),
            Some("0.1.0-rc.10"),
            &latest,
            &[],
        );
        assert_eq!(decision, UpdateCheck::HealUpToDate);
    }

    #[test]
    fn resolve_legacy_tag_match_latest_tag_is_up_to_date() {
        let latest = latest(
            "dsh-0.1.2-rc.1-33729514615",
            "33729514615", // 限流兜底
        );
        let tags = vec![(
            "dsh-0.1.2-rc.1-33729514615".to_string(),
            "abc123def4567890abcdef1234567890abcdef12".to_string(),
        )];
        let decision = resolve_update(
            Some("abc123def4567890abcdef1234567890abcdef12"),
            None,
            Some("0.1.2-rc.1"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::UpToDate);
    }

    #[test]
    fn resolve_legacy_tag_older_build_is_still_hotfix_update() {
        let latest = latest("dsh-0.1.2-rc.1-33729514615", "33729514615");
        let tags = vec![(
            "dsh-0.1.2-rc.1-33600000000".to_string(), // 旧 build，不是 latest
            "11122233344455556666777788899900aaaabbbb".to_string(),
        )];
        let decision = resolve_update(
            Some("11122233344455556666777788899900aaaabbbb"),
            None,
            Some("0.1.2-rc.1"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }
    #[test]
    fn resolve_legacy_tag_match_not_first_in_list_is_up_to_date() {
        let latest = latest("dsh-0.1.2-rc.1-33729514615", "33729514615");
        let tags = vec![
            (
                "unrelated/v0.0.1".to_string(),
                "fff000fff000fff000fff000fff000fff000f0000".to_string(),
            ),
            (
                "dsh-0.1.2-rc.1-33729514615".to_string(),
                "abc123def4567890abcdef1234567890abcdef12".to_string(),
            ),
        ];
        let decision = resolve_update(
            Some("abc123def4567890abcdef1234567890abcdef12"),
            None,
            Some("0.1.2-rc.1"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::UpToDate);
    }

    #[test]
    fn resolve_legacy_tag_multiple_matches_is_update_available() {
        let latest = latest("dsh-0.1.2-rc.1-33729514615", "33729514615");
        let tags = vec![
            (
                "dsh-0.1.2-rc.1-stable".to_string(),
                "abc123def4567890abcdef1234567890abcdef12".to_string(),
            ),
            (
                "dsh-0.1.2-rc.1-33729514615".to_string(),
                "abc123def4567890abcdef1234567890abcdef12".to_string(),
            ),
        ];
        let decision = resolve_update(
            Some("abc123def4567890abcdef1234567890abcdef12"),
            None,
            Some("0.1.2-rc.1"),
            &latest,
            &tags,
        );
        assert_eq!(decision, UpdateCheck::UpdateAvailable);
    }
}
