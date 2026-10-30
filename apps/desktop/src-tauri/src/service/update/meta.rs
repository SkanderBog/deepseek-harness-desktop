use std::time::Duration;

use super::REPO_URL;
use super::version::{current_version, is_newer, is_stable, parse_version, pick_asset};
use crate::AppHandle;

#[derive(Debug, Clone)]
pub(super) struct LatestRelease {
    pub(super) version: String,
    pub(super) tag: String,
    pub(super) published_at: String,
    pub(super) url: String,
    pub(super) asset_name: String,
    pub(super) digest: Option<String>,
}

fn http_client(app_handle: &AppHandle) -> Result<reqwest::Client, String> {
    crate::config::proxy::http_client_builder(app_handle)?
        .user_agent("deepseek-harness-desktop")
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("UPDATE_CLIENT: {e}"))
}

fn find_token<'a>(s: &'a str, marker: &str, end_marker: &str) -> Option<&'a str> {
    let start = s.find(marker)? + marker.len();
    let end = s[start..].find(end_marker).map(|e| start + e)?;
    Some(&s[start..end])
}

fn parse_atom_entries(body: &str) -> Vec<(String, String)> {
    let mut releases = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<entry>") {
        let end = rest[start..]
            .find("</entry>")
            .map(|e| start + e)
            .unwrap_or(rest.len());
        let block = &rest[start..end];
        if let Some(tag) = find_token(block, "releases/tag/", "\"") {
            let published_at = find_token(block, "<updated>", "</updated>")
                .unwrap_or_default()
                .to_string();
            releases.push((tag.to_string(), published_at));
        }
        rest = &rest[end..];
    }
    releases
}

pub(super) async fn fetch_releases_meta(
    app_handle: &AppHandle,
) -> Result<Vec<(String, String)>, String> {
    let body = http_client(app_handle)?
        .get(format!("{REPO_URL}/releases.atom"))
        .send()
        .await
        .map_err(|e| format!("UPDATE_ATOM: {e}"))?
        .error_for_status()
        .map_err(|e| format!("UPDATE_ATOM: {e}"))?
        .text()
        .await
        .map_err(|e| format!("UPDATE_ATOM: {e}"))?;
    let releases = parse_atom_entries(&body);
    if releases.is_empty() {
        return Err("UPDATE_PARSE: no releases found in atom feed".to_string());
    }
    Ok(releases)
}

fn extract_asset_names(html: &str, tag: &str) -> Vec<String> {
    let needle = format!("releases/download/{tag}/");
    let mut names = Vec::new();
    let mut start = 0;
    while let Some(pos) = html[start..].find(&needle) {
        let after = start + pos + needle.len();
        let end = html[after..]
            .find('"')
            .map(|e| after + e)
            .unwrap_or(html.len());
        names.push(html[after..end].to_string());
        start = end;
    }
    names
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

async fn fetch_expanded_assets(
    app_handle: &AppHandle,
    tag: &str,
) -> Result<(Vec<String>, String), String> {
    let body = http_client(app_handle)?
        .get(format!("{REPO_URL}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .map_err(|e| format!("UPDATE_ASSETS: {e}"))?
        .error_for_status()
        .map_err(|e| format!("UPDATE_ASSETS: {e}"))?
        .text()
        .await
        .map_err(|e| format!("UPDATE_ASSETS: {e}"))?;
    let names = extract_asset_names(&body, tag);
    Ok((names, body))
}

fn stable_candidates(
    releases: &[(String, String)],
    current: &str,
) -> (Vec<(String, String)>, bool) {
    let mut candidates = Vec::new();
    let mut saw_stable = false;
    for (tag, published_at) in releases {
        let version = tag.trim_start_matches('v');
        let Some(parsed) = parse_version(version) else {
            log::debug!("UPDATE_SKIP: {tag} 非法 semver（手动测试 release?），跳过");
            continue;
        };
        if !is_stable(&parsed) {
            log::debug!("UPDATE_SKIP: {tag} 为 pre-release（非正式版），不通知用户");
            continue;
        }
        saw_stable = true;
        if !is_newer(version, current) {
            log::debug!("UPDATE_SKIP: {tag} 不高于当前版本 {current}");
            continue;
        }
        candidates.push((tag.clone(), published_at.clone()));
    }
    (candidates, saw_stable)
}

fn is_stable_tag(tag: &str) -> bool {
    parse_version(tag.trim_start_matches('v')).is_some_and(|parsed| is_stable(&parsed))
}

fn parse_latest_tag(url: &str) -> Option<String> {
    const MARKER: &str = "/releases/tag/";
    let rest = &url[url.find(MARKER)? + MARKER.len()..];
    let end = rest
        .find(|c| matches!(c, '?' | '#' | '/'))
        .unwrap_or(rest.len());
    let tag = &rest[..end];
    (!tag.is_empty()).then(|| tag.to_string())
}

fn parse_release_listing_tags(html: &str) -> Vec<String> {
    const MARKER: &str = "/releases/tag/";
    let mut tags: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(offset) = html[from..].find(MARKER) {
        let start = from + offset + MARKER.len();
        let tail = &html[start..];
        let end = tail
            .find(|c: char| matches!(c, '"' | '\'' | '?' | '#' | '>'))
            .unwrap_or(tail.len());
        let tag = &tail[..end];
        if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
        from = start + end;
    }
    tags
}

fn listed_stable_candidates(
    tags: &[String],
    current: &str,
    skip: &[String],
) -> Vec<(String, String)> {
    tags.iter()
        .filter(|tag| !skip.iter().any(|s| tag == &s))
        .filter(|tag| is_stable_tag(tag) && is_newer(tag.trim_start_matches('v'), current))
        .map(|tag| (tag.clone(), String::new()))
        .collect()
}

fn is_iso_utc(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && value.ends_with('Z')
        && value[..19]
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit())
}

fn parse_published_at(html: &str) -> Option<String> {
    const MARKER: &str = "datetime=\"";
    let mut rest = html;
    while let Some(pos) = rest.find(MARKER) {
        let tail = &rest[pos + MARKER.len()..];
        let end = tail.find('"')?;
        let value = &tail[..end];
        if is_iso_utc(value) {
            return Some(value.to_string());
        }
        rest = &tail[end..];
    }
    None
}

async fn fetch_latest_stable_tag(app_handle: &AppHandle) -> Result<Option<String>, String> {
    let response = http_client(app_handle)?
        .get(format!("{REPO_URL}/releases/latest"))
        .send()
        .await
        .map_err(|e| format!("UPDATE_LATEST: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("UPDATE_LATEST: {e}"))?;
    Ok(parse_latest_tag(response.url().as_str()))
}

async fn fetch_listed_stable_candidates(
    app_handle: &AppHandle,
    current: &str,
    skip: &[String],
) -> Result<Vec<(String, String)>, String> {
    const LISTING_PAGES: u32 = 3;
    let client = http_client(app_handle)?;
    let mut found: Vec<(String, String)> = Vec::new();
    for page in 1..=LISTING_PAGES {
        let body = client
            .get(format!("{REPO_URL}/releases?page={page}"))
            .send()
            .await
            .map_err(|e| format!("UPDATE_LISTING: {e}"))?
            .error_for_status()
            .map_err(|e| format!("UPDATE_LISTING: {e}"))?
            .text()
            .await
            .map_err(|e| format!("UPDATE_LISTING: {e}"))?;
        let tags = parse_release_listing_tags(&body);
        if tags.is_empty() {
            break;
        }
        found.extend(listed_stable_candidates(&tags, current, skip));
        if !found.is_empty() {
            break;
        }
    }
    Ok(found)
}

pub(super) async fn fetch_tag_published_at(
    app_handle: &AppHandle,
    tag: &str,
) -> Result<Option<String>, String> {
    let body = http_client(app_handle)?
        .get(format!("{REPO_URL}/releases/tag/{tag}"))
        .send()
        .await
        .map_err(|e| format!("UPDATE_TAG: {e}"))?
        .error_for_status()
        .map_err(|e| format!("UPDATE_TAG: {e}"))?
        .text()
        .await
        .map_err(|e| format!("UPDATE_TAG: {e}"))?;
    Ok(parse_published_at(&body))
}

async fn first_matching_release(
    app_handle: &AppHandle,
    candidates: Vec<(String, String)>,
) -> Result<Option<LatestRelease>, String> {
    for (tag, published_at) in candidates {
        let version = tag.trim_start_matches('v');
        if let Some(release) =
            fetch_release_assets(app_handle, &tag, version, &published_at).await?
        {
            return Ok(Some(release));
        }
    }
    Ok(None)
}

pub(super) async fn fetch_latest_release(
    app_handle: &AppHandle,
) -> Result<Option<LatestRelease>, String> {
    if !super::version::supports_stable_updates(&app_handle.config().identifier) {
        return Ok(None);
    }
    let current = current_version();
    let (candidates, saw_stable) =
        stable_candidates(&fetch_releases_meta(app_handle).await?, &current);

    if saw_stable {
        return first_matching_release(app_handle, candidates).await;
    }

    let mut fallback: Vec<(String, String)> = Vec::new();
    if let Some(tag) = fetch_latest_stable_tag(app_handle).await? {
        if is_stable_tag(&tag) && is_newer(tag.trim_start_matches('v'), &current) {
            fallback.push((tag, String::new()));
        } else {
            log::debug!("UPDATE_SKIP: {tag} 非可用正式版（非法/预发布/不高于 {current}）");
        }
    }
    if fallback.is_empty() {
        return Ok(None);
    }
    let skip: Vec<String> = fallback.iter().map(|(tag, _)| tag.clone()).collect();
    if let Some(release) = first_matching_release(app_handle, fallback).await? {
        return Ok(Some(release));
    }

    let older = fetch_listed_stable_candidates(app_handle, &current, &skip).await?;
    first_matching_release(app_handle, older).await
}

async fn fetch_release_assets(
    app_handle: &AppHandle,
    tag: &str,
    version: &str,
    published_at: &str,
) -> Result<Option<LatestRelease>, String> {
    let (names, body) = fetch_expanded_assets(app_handle, tag).await?;
    let Some(asset_name) = pick_asset(&names) else {
        log::debug!("UPDATE_SKIP: {tag} 无当前平台安装包，继续看更旧的正式版");
        return Ok(None);
    };

    let digest = parse_digest_from_expanded_assets(&body, &asset_name);
    log::debug!(
        "Release {tag} digest for picked asset {}: {}",
        asset_name,
        digest.as_deref().map(|d| &d[..12]).unwrap_or("<none>")
    );

    let url = format!("{REPO_URL}/releases/download/{tag}/{asset_name}");
    Ok(Some(LatestRelease {
        version: version.to_string(),
        tag: tag.to_string(),
        published_at: published_at.to_string(),
        url,
        asset_name,
        digest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_token_extracts_between_markers() {
        let s = r#"<link rel="alternate" href="https://github.com/x/releases/tag/v0.6.6"/>"#;
        assert_eq!(find_token(s, "releases/tag/", "\""), Some("v0.6.6"));
        let s2 = "<updated>2026-08-19T09:27:38Z</updated>";
        assert_eq!(
            find_token(s2, "<updated>", "</updated>"),
            Some("2026-08-19T09:27:38Z")
        );
        assert_eq!(find_token("no marker", "releases/tag/", "\""), None);
    }

    #[test]
    fn parse_atom_entries_multiple_entries_in_order() {
        let feed = r#"<feed>
            <entry><id>1</id><link rel="alternate" href="/hairyf/deepseek-harness-desktop/releases/tag/v0.7.14-rc.1"/><updated>2026-08-20T01:00:00Z</updated></entry>
            <entry><id>2</id><link rel="alternate" href="/hairyf/deepseek-harness-desktop/releases/tag/v0.7.13"/><updated>2026-08-19T00:00:00Z</updated></entry>
            <entry><id>3</id><link rel="alternate" href="/hairyf/deepseek-harness-desktop/releases/tag/test-main-42"/></entry>
        </feed>"#;
        assert_eq!(
            parse_atom_entries(feed),
            vec![
                (
                    "v0.7.14-rc.1".to_string(),
                    "2026-08-20T01:00:00Z".to_string()
                ),
                ("v0.7.13".to_string(), "2026-08-19T00:00:00Z".to_string()),
                ("test-main-42".to_string(), String::new()),
            ]
        );
        assert!(parse_atom_entries("").is_empty());
        assert!(parse_atom_entries("<feed></feed>").is_empty());
    }

    #[test]
    fn extract_asset_names_parses_download_links() {
        let tag = "v0.6.6";
        let html = r#"
            <a href="/hairyf/deepseek-harness-desktop/releases/download/v0.6.6/x64-setup.exe">x</a>
            <a href="/hairyf/deepseek-harness-desktop/releases/download/v0.6.6/x64_en-US.msi">y</a>
            <a href="/hairyf/deepseek-harness-desktop/releases/download/v0.6.5/old.dmg">z</a>
        "#;
        let names = extract_asset_names(html, tag);
        assert_eq!(names, vec!["x64-setup.exe", "x64_en-US.msi"]);
        assert!(extract_asset_names(html, "v9.9.9").is_empty());
        assert!(extract_asset_names("", tag).is_empty());
    }

    #[test]
    fn parse_digest_from_expanded_assets_extracts_sha256() {
        let hex = format!("sha256:{}", "a".repeat(64));
        let html = format!(
            r#"<td>设置包</td><td class="d-block">app.dmg</td><td>下载</td><td>{hex}</td>"#
        );
        let digest = parse_digest_from_expanded_assets(&html, "app.dmg");
        let expected = format!("sha256:{}", "a".repeat(64));
        assert_eq!(digest.as_deref(), Some(expected.as_str()));

        assert!(parse_digest_from_expanded_assets(&html, "app-x86_64.dmg").is_none());
        let bad = r#"<td>app.dmg sha256:zz"#;
        assert!(parse_digest_from_expanded_assets(bad, "app.dmg").is_none());
        let unicode = format!("中文说明app.dmg{}更多内容", hex);
        assert!(parse_digest_from_expanded_assets(&unicode, "app.dmg").is_some());
    }

    #[test]
    fn digest_is_resolved_per_picked_asset_not_first_in_page() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let body = format!(
            r#"<a href="/x/y/releases/download/v0.7.5/app.rpm">app.rpm</a><span>sha256:{a}</span>
               <a href="/x/y/releases/download/v0.7.5/setup.exe">setup.exe</a><span>sha256:{b}</span>"#
        );
        let rpm_digest = parse_digest_from_expanded_assets(&body, "app.rpm");
        let picked_digest = parse_digest_from_expanded_assets(&body, "setup.exe");
        let expected_rpm = format!("sha256:{a}");
        let expected_picked = format!("sha256:{b}");
        assert_eq!(rpm_digest.as_deref(), Some(expected_rpm.as_str()));
        assert_eq!(picked_digest.as_deref(), Some(expected_picked.as_str()));
        assert_ne!(rpm_digest, picked_digest);
    }

    #[test]
    fn stable_candidates_reports_window_without_stable() {
        let nightlies: Vec<(String, String)> = (1..=10)
            .map(|day| {
                (
                    format!("nightly-202610{day:02}"),
                    format!("2026-10-{day:02}T18:00:00Z"),
                )
            })
            .collect();
        let (candidates, saw_stable) = stable_candidates(&nightlies, "0.22.3");
        assert!(candidates.is_empty());
        assert!(!saw_stable);

        let mut mixed = nightlies.clone();
        mixed.insert(
            0,
            ("nightly".to_string(), "2026-10-11T18:00:00Z".to_string()),
        );
        let (candidates, saw_stable) = stable_candidates(&mixed, "0.22.3");
        assert!(candidates.is_empty());
        assert!(!saw_stable);

        mixed.push(("v0.22.4".to_string(), "2026-10-12T00:00:00Z".to_string()));
        let (candidates, saw_stable) = stable_candidates(&mixed, "0.22.3");
        assert_eq!(
            candidates,
            vec![("v0.22.4".to_string(), "2026-10-12T00:00:00Z".to_string())]
        );
        assert!(saw_stable);
    }

    #[test]
    fn stable_candidates_skips_prerelease_and_junk_tags() {
        let releases = vec![
            ("v0.23.0-rc.1".to_string(), "t1".to_string()),
            ("test-main-42".to_string(), "t2".to_string()),
            ("v0.22.3".to_string(), "t3".to_string()),
            ("v0.22.2".to_string(), "t4".to_string()),
        ];
        let (candidates, saw_stable) = stable_candidates(&releases, "0.22.3");
        assert!(candidates.is_empty());
        assert!(saw_stable);

        let (candidates, saw_stable) = stable_candidates(&releases, "0.22.2");
        assert_eq!(candidates, vec![("v0.22.3".to_string(), "t3".to_string())]);
        assert!(saw_stable);
    }

    #[test]
    fn parse_latest_tag_reads_redirect_target() {
        assert_eq!(
            parse_latest_tag("https://github.com/x/y/releases/tag/v0.22.3").as_deref(),
            Some("v0.22.3")
        );
        assert_eq!(
            parse_latest_tag("https://github.com/x/y/releases/tag/v0.22.3?expanded=true")
                .as_deref(),
            Some("v0.22.3")
        );
        assert_eq!(
            parse_latest_tag("https://github.com/x/y/releases/latest"),
            None
        );
        assert_eq!(
            parse_latest_tag("https://github.com/x/y/releases/tag/"),
            None
        );
    }

    #[test]
    fn parse_release_listing_tags_extracts_unique_tags_in_order() {
        let html = r#"
            <a href="/x/y/releases/tag/nightly-20261007">n</a>
            <a href="/x/y/releases/tag/nightly">alias</a>
            <a href="/x/y/releases/tag/v0.22.3">a</a>
            <a href="/x/y/releases/tag/v0.22.3">dup</a>
            <a href="/x/y/releases/tag/v0.22.2?expanded=true">b</a>
            <a href="/x/y/releases/download/v0.22.2/x.dmg">asset</a>
        "#;
        assert_eq!(
            parse_release_listing_tags(html),
            vec!["nightly-20261007", "nightly", "v0.22.3", "v0.22.2"]
        );
        assert!(parse_release_listing_tags("").is_empty());
        assert!(parse_release_listing_tags(r#"<a href="/x/y/releases">list</a>"#).is_empty());
        assert!(parse_release_listing_tags(r#"<a href="/x/y/releases/tag/">x</a>"#).is_empty());
    }

    #[test]
    fn listed_stable_candidates_filters_and_skips_tried_tags() {
        let tags: Vec<String> = [
            "nightly-20261007",
            "nightly",
            "v0.23.0-rc.1",
            "test-main-42",
            "v0.22.3",
            "v0.22.2",
            "v0.22.1",
        ]
        .iter()
        .map(|tag| tag.to_string())
        .collect();
        let skip = vec!["v0.22.3".to_string()];
        assert_eq!(
            listed_stable_candidates(&tags, "0.22.0", &skip),
            vec![
                ("v0.22.2".to_string(), String::new()),
                ("v0.22.1".to_string(), String::new()),
            ]
        );
        assert!(listed_stable_candidates(&tags, "0.22.3", &skip).is_empty());
    }

    #[test]
    fn parse_published_at_skips_display_only_datetimes() {
        let html = r#"<relative-time datetime="2026-10-06T16:11:01Z">last week</relative-time>
            <relative-time datetime="2026-10-06 15:28:28 UTC">committed</relative-time>"#;
        assert_eq!(
            parse_published_at(html).as_deref(),
            Some("2026-10-06T16:11:01Z")
        );
        assert_eq!(parse_published_at("<p>no datetime</p>"), None);
        assert_eq!(
            parse_published_at(r#"<p datetime="2026-10-06 15:28:28 UTC">x</p>"#),
            None
        );
    }

    #[tokio::test]
    #[ignore = "联网测试：需手动 --ignored 运行，会访问真实 GitHub 页面"]
    async fn live_github_release_pages_are_parseable() {
        let client = reqwest::Client::builder()
            .user_agent("dsh-tauri-update-check-test")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("构建测试用 HTTP 客户端失败");
        let fetch = |url: String| {
            let client = client.clone();
            async move {
                let response = client.get(url).send().await.expect("请求失败");
                assert!(response.status().is_success(), "HTTP {}", response.status());
                response.text().await.expect("读取正文失败")
            }
        };

        let feed = fetch(format!("{REPO_URL}/releases.atom")).await;
        let entries = parse_atom_entries(&feed);
        assert!(
            !entries.is_empty(),
            "atom feed 解析不出 entry：页面结构可能已变化"
        );
        let (candidates, _) = stable_candidates(&entries, "0.0.0");
        assert!(
            candidates.iter().all(|(tag, _)| is_stable_tag(tag)),
            "候选里混进了非正式版：{candidates:?}"
        );

        let response = client
            .get(format!("{REPO_URL}/releases/latest"))
            .send()
            .await
            .expect("请求 /releases/latest 失败");
        assert!(response.status().is_success(), "HTTP {}", response.status());
        let tag = parse_latest_tag(response.url().as_str())
            .expect("/releases/latest 未重定向到 tag 页面");
        assert!(is_stable_tag(&tag), "{tag} 不是正式版");

        let page = fetch(format!("{REPO_URL}/releases/tag/{tag}")).await;
        assert!(
            parse_published_at(&page).is_some(),
            "解析不出 {tag} 的发布时间"
        );

        let assets = fetch(format!("{REPO_URL}/releases/expanded_assets/{tag}")).await;
        let names = extract_asset_names(&assets, &tag);
        assert!(!names.is_empty(), "{tag} 的资产页解析不出文件名");

        let listing = fetch(format!("{REPO_URL}/releases?page=1")).await;
        let listed =
            listed_stable_candidates(&parse_release_listing_tags(&listing), "0.0.0", &[tag]);
        assert!(!listed.is_empty(), "列表页里找不到更旧的正式版");
    }
}
