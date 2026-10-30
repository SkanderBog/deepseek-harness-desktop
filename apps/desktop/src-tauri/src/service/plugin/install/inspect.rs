use std::path::Path;
use std::time::Duration;

use semver::Version;

use crate::service::plugin::compat::{self, PluginInspect};
use crate::service::plugin::update::encode_registry_name;

use super::spec;
use crate::AppHandle;

const INSPECT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn inspect_specs(
    app_handle: &AppHandle,
    specs: &[String],
    dsh: Option<String>,
) -> Result<Vec<PluginInspect>, String> {
    let runtime = dsh
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| crate::service::core::active_version(app_handle));
    let client = crate::config::proxy::http_client_builder(app_handle)?
        .timeout(INSPECT_TIMEOUT)
        .build()
        .map_err(|e| format!("INSPECT_CLIENT: {e}"))?;

    let base = crate::service::core::active_core_dir(app_handle);
    let mut results = Vec::with_capacity(specs.len());
    for raw in specs {
        results.push(inspect_one(&client, raw, &base, runtime.as_deref()).await);
    }
    Ok(results)
}

pub(crate) async fn packument(app_handle: &AppHandle, name: &str) -> Option<serde_json::Value> {
    let client = crate::config::proxy::http_client_builder(app_handle)
        .ok()?
        .timeout(INSPECT_TIMEOUT)
        .build()
        .ok()?;
    let url = format!("https://registry.npmjs.org/{}", encode_registry_name(name));
    let response = client
        .get(&url)
        .header("accept", "application/vnd.npm.install-v1+json")
        .header("user-agent", "deepseek-harness-desktop")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<serde_json::Value>().await.ok()
}

async fn inspect_one(
    client: &reqwest::Client,
    spec: &str,
    base: &Path,
    runtime: Option<&str>,
) -> PluginInspect {
    let raw = spec.trim();
    if spec::local_path_spec(raw).is_some() {
        return inspect_local(raw, base, runtime);
    }
    let Some(name) = spec::package_name_of_spec(raw, base) else {
        return uninspected(raw, None);
    };

    let requested = requested_version(raw, &name);
    let exact = requested
        .as_deref()
        .filter(|value| Version::parse(value).is_ok());
    let target = exact.unwrap_or("latest");
    let url = format!(
        "https://registry.npmjs.org/{}/{}",
        encode_registry_name(&name),
        target
    );
    let response = match client
        .get(&url)
        .header("accept", "application/json")
        .header("user-agent", "deepseek-harness-desktop")
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return problem_result(raw, Some(name), "network"),
    };
    if !response.status().is_success() {
        let problem = if response.status() == reqwest::StatusCode::NOT_FOUND {
            "not-found"
        } else {
            "network"
        };
        return problem_result(raw, Some(name), problem);
    }
    let Ok(manifest) = response.json::<serde_json::Value>().await else {
        return problem_result(raw, Some(name), "network");
    };

    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(String::from);
    let peers = compat::peers_from_manifest(&manifest);
    let compatible = if requested.is_some() && exact.is_none() {
        None
    } else {
        runtime.and_then(|runtime| compat::evaluate(&peers, runtime))
    };
    PluginInspect {
        spec: raw.to_string(),
        name: Some(name),
        version,
        compatible,
        peers: (!peers.is_empty()).then_some(peers),
        problem: None,
    }
}

fn inspect_local(raw: &str, base: &Path, runtime: Option<&str>) -> PluginInspect {
    let Some(dir) = spec::local_dir_of(raw, base) else {
        return problem_result(raw, None, "local-missing");
    };
    let Ok(content) = std::fs::read_to_string(dir.join("package.json")) else {
        return problem_result(raw, None, "local-missing");
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) else {
        return problem_result(raw, None, "local-missing");
    };
    let name = manifest
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .or_else(|| spec::local_package_name(&dir));
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(String::from);
    let peers = compat::peers_from_manifest(&manifest);
    let compatible = runtime.and_then(|runtime| compat::evaluate(&peers, runtime));
    PluginInspect {
        spec: raw.to_string(),
        name,
        version,
        compatible,
        peers: (!peers.is_empty()).then_some(peers),
        problem: None,
    }
}

fn requested_version(raw: &str, name: &str) -> Option<String> {
    raw.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('@'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn problem_result(spec: &str, name: Option<String>, problem: &str) -> PluginInspect {
    PluginInspect {
        problem: Some(problem.to_string()),
        ..uninspected(spec, name)
    }
}

fn uninspected(spec: &str, name: Option<String>) -> PluginInspect {
    PluginInspect {
        spec: spec.to_string(),
        name,
        version: None,
        compatible: None,
        peers: None,
        problem: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(tag: &str, manifest: Option<&str>) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("dsh-inspect-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("plugin");
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(manifest) = manifest {
            std::fs::write(dir.join("package.json"), manifest).unwrap();
        }
        let base = dunce::simplified(&std::fs::canonicalize(&base).unwrap()).to_path_buf();
        let dir = base.join("plugin");
        (base, dir)
    }

    #[test]
    fn local_spec_is_inspected_offline_from_its_manifest() {
        let (base, dir) = fixture(
            "ok",
            Some(
                r#"{"name":"dsh-reverse-skill","version":"1.0.2","peerDependencies":{"@deepseek-ai/cordis":"^4.0.4"}}"#,
            ),
        );

        let result = inspect_local("./plugin", &base, Some("0.2.0-rc.2"));

        assert_eq!(result.spec, "./plugin");
        assert_eq!(result.name.as_deref(), Some("dsh-reverse-skill"));
        assert_eq!(result.version.as_deref(), Some("1.0.2"));
        assert_eq!(result.problem, None);
        assert_eq!(result.compatible, None, "非 DSH 家族 peer：无从判定");
        assert!(result.peers.is_none());
        assert_eq!(
            spec::local_dir_of("./plugin", &base).as_deref(),
            Some(dir.as_path())
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_spec_reports_dsh_peer_compatibility_without_network() {
        let (base, _) = fixture(
            "peer",
            Some(r#"{"name":"dsh-probe","peerDependencies":{"@deepseek-ai/dsh":"^0.2.0"}}"#),
        );

        let compatible = inspect_local("./plugin", &base, Some("0.2.1"));
        assert_eq!(compatible.compatible, Some(true));
        assert_eq!(
            compatible
                .peers
                .as_ref()
                .and_then(|p| p.get("@deepseek-ai/dsh"))
                .map(String::as_str),
            Some("^0.2.0")
        );

        let incompatible = inspect_local("./plugin", &base, Some("0.1.7"));
        assert_eq!(incompatible.compatible, Some(false));

        let unknown = inspect_local("./plugin", &base, None);
        assert_eq!(unknown.compatible, None);
        assert!(unknown.peers.is_some());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_spec_reports_local_missing_for_absent_dir_or_manifest() {
        let (base, dir) = fixture("absent", None);

        let missing_dir = inspect_local("./never-existed", &base, Some("0.2.0"));
        assert_eq!(missing_dir.problem.as_deref(), Some("local-missing"));
        assert_eq!(missing_dir.name, None);
        assert_eq!(missing_dir.compatible, None);

        let missing_manifest = inspect_local("./plugin", &base, Some("0.2.0"));
        assert_eq!(missing_manifest.problem.as_deref(), Some("local-missing"));

        std::fs::write(dir.join("package.json"), "{ not json").unwrap();
        let broken = inspect_local("./plugin", &base, Some("0.2.0"));
        assert_eq!(broken.problem.as_deref(), Some("local-missing"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_spec_name_falls_back_to_the_directory_name() {
        let (base, _) = fixture("noname", Some(r#"{"version":"1.0.0"}"#));

        let result = inspect_local("./plugin", &base, Some("0.2.0"));

        assert_eq!(result.problem, None);
        assert_eq!(result.name.as_deref(), Some("plugin"));
        assert_eq!(result.version.as_deref(), Some("1.0.0"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn non_registry_specs_are_left_uninspected() {
        let client = reqwest::Client::new();
        let base = std::path::Path::new(".");
        for spec in [
            "github:MengYuil/dsh-ponytail",
            "https://github.com/MengYuil/dsh-ponytail",
            "git+https://github.com/MengYuil/dsh-ponytail.git#main",
            "https://example.com/plugin.tgz",
        ] {
            let result = inspect_one(&client, spec, base, Some("0.2.0")).await;
            assert_eq!(result.spec, spec);
            assert_eq!(result.problem, None, "{spec}");
            assert_eq!(result.compatible, None, "{spec}");
            assert_eq!(result.name, None, "{spec}");
        }
    }
}
