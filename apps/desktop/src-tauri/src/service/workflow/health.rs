
use std::sync::atomic::Ordering;

use futures_util::StreamExt;

use crate::config;

use super::process::{has_owned_process, launch_guard};
use super::startup;
use super::utils;

const HEALTH_PROBE_CONCURRENCY: usize = 48;

#[derive(serde::Serialize)]
pub struct HealthCheck {
    pub status: String,
    pub boot_html: String,
    pub service_url: String,
}

async fn client_probe_endpoints(port: u16) -> Result<(Vec<String>, String), String> {
    if !utils::is_port_in_use(port) {
        return Err(format!(
            "HARNESS_NOT_READY: Harness service is not listening yet (port {port})"
        ));
    }
    let client = utils::loopback_http_client(config::HEALTH_CHECK_TIMEOUT)
        .map_err(|e| format!("HARNESS_HEALTH_CLIENT_FAILED: {e}"))?;
    let root = format!("{}/", config::get_dsh_service_url(port));
    let started = std::time::Instant::now();
    let response = client.get(root).send().await.map_err(|e| {
        log::warn!(
            "Harness boot manifest request failed: elapsed_ms={} connect={} timeout={} {e:?}",
            started.elapsed().as_millis(),
            e.is_connect(),
            e.is_timeout(),
        );
        format!(
            "HARNESS_BOOT_MANIFEST_REQUEST_FAILED: connect={} timeout={} {e:?}",
            e.is_connect(),
            e.is_timeout(),
        )
    })?;
    startup::note_http_answer();
    if !response.status().is_success() {
        return Err(format!(
            "HARNESS_NOT_READY: boot page returned {}",
            response.status()
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|e| format!("HARNESS_BOOT_MANIFEST_READ_FAILED: {e}"))?;
    let endpoints = utils::client_urls_from_boot_html(port, &body)
        .unwrap_or_else(|| utils::health_probe_plugin_urls(port));
    Ok((endpoints, body))
}

fn not_owned_probe_signal(launch_in_progress: bool) -> &'static str {
    if launch_in_progress {
        "HARNESS_NOT_READY: Harness service is still starting"
    } else {
        "HARNESS_NOT_OWNED: no Harness process is owned by this app"
    }
}

fn all_client_modules_ready(ready: usize, total: usize) -> bool {
    total > 0 && ready == total
}

async fn probe_client_bundles(
    client: &reqwest::Client,
    endpoints: Vec<String>,
) -> (usize, Vec<String>) {
    let mut outcomes = futures_util::stream::iter(endpoints.into_iter().enumerate())
        .map(|(index, endpoint)| async move {
            let failure = match client.get(&endpoint).send().await {
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if utils::looks_like_plugin_bundle(status.is_success(), &body) {
                        None
                    } else {
                        let failure = format!("{endpoint} returned {status} (not a plugin bundle)");
                        log::debug!("Health check failed: {failure}");
                        Some(failure)
                    }
                }
                Err(err) => {
                    log::debug!("Health check {endpoint}: {err}");
                    Some(format!("{endpoint}: {err}"))
                }
            };
            (index, failure)
        })
        .buffer_unordered(HEALTH_PROBE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    outcomes.sort_unstable_by_key(|(index, _)| *index);

    let mut ready = 0usize;
    let mut failures = Vec::with_capacity(outcomes.len());
    for (_, failure) in outcomes {
        match failure {
            Some(failure) => failures.push(failure),
            None => ready += 1,
        }
    }
    (ready, failures)
}

pub async fn proxy_health_check(port: u16) -> Result<HealthCheck, String> {
    if !has_owned_process() {
        return Err(not_owned_probe_signal(launch_guard().load(Ordering::SeqCst)).to_string());
    }
    let client = utils::loopback_http_client(config::HEALTH_CHECK_TIMEOUT)
        .map_err(|e| format!("HARNESS_HEALTH_CLIENT_FAILED: {e}"))?;
    let (endpoints, boot_html) = client_probe_endpoints(port).await?;
    let total = endpoints.len();
    let (ready, failures) = probe_client_bundles(&client, endpoints).await;
    if all_client_modules_ready(ready, total) {
        startup::note_client_modules_ready(ready, total);
        return Ok(HealthCheck {
            status: format!("healthy - {ready}/{total} client modules ready"),
            boot_html,
            service_url: config::get_dsh_service_url(port),
        });
    }
    Err(format!(
        "HARNESS_NOT_READY: Harness client modules are not ready ({ready}/{total} ready; {})",
        failures.join("; ")
    ))
}

fn ownership_recheck_signal(owned: bool, launch_in_progress: bool) -> Result<(), String> {
    if owned || launch_in_progress {
        return Ok(());
    }
    Err(not_owned_probe_signal(false).to_string())
}

pub fn recheck_ownership() -> Result<(), String> {
    ownership_recheck_signal(has_owned_process(), launch_guard().load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_owned_is_retryable_during_launch_not_fatal() {
        assert!(not_owned_probe_signal(true).starts_with("HARNESS_NOT_READY"));
        assert!(not_owned_probe_signal(false).starts_with("HARNESS_NOT_OWNED"));
    }

    #[test]
    fn ownership_recheck_only_fails_when_process_is_gone() {
        assert!(ownership_recheck_signal(true, false).is_ok());
        assert!(ownership_recheck_signal(true, true).is_ok());
        assert!(ownership_recheck_signal(false, true).is_ok());
        assert_eq!(
            ownership_recheck_signal(false, false).unwrap_err(),
            "HARNESS_NOT_OWNED: no Harness process is owned by this app"
        );
    }

    #[tokio::test]
    async fn boot_probe_reports_unlistening_port_without_waiting_for_connect_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let started = std::time::Instant::now();
        let error = client_probe_endpoints(port).await.unwrap_err();
        let elapsed = started.elapsed();

        assert_eq!(
            error,
            format!("HARNESS_NOT_READY: Harness service is not listening yet (port {port})")
        );
        assert!(
            elapsed < std::time::Duration::from_millis(150),
            "unlistening-port probe waited {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn boot_probe_still_reports_response_timeout_when_port_is_listening() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, hold) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                let _ = hold.await;
                drop(socket);
            }
        });

        let result = client_probe_endpoints(port).await;
        let _ = release.send(());
        server.await.unwrap();
        let error = result.unwrap_err();
        assert!(
            error.starts_with("HARNESS_BOOT_MANIFEST_REQUEST_FAILED:"),
            "{error}"
        );
        assert!(error.contains("connect=false"), "{error}");
        assert!(error.contains("timeout=true"), "{error}");
    }

    #[tokio::test]
    async fn boot_probe_distinguishes_response_timeout_from_connection_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, mut hold) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            tokio::select! {
                socket = listener.accept() => {
                    let (socket, _) = socket.unwrap();
                    let _ = hold.await;
                    drop(socket);
                }
                _ = &mut hold => {}
            }
        });

        let result = client_probe_endpoints(port).await;
        let _ = release.send(());
        server.await.unwrap();
        let error = result.unwrap_err();
        assert!(
            error.starts_with("HARNESS_BOOT_MANIFEST_REQUEST_FAILED:"),
            "{error}"
        );
        assert!(error.contains("connect=false"), "{error}");
        assert!(error.contains("timeout=true"), "{error}");
        assert!(!error.contains("elapsed_ms="), "{error}");
        assert!(error.contains("source:"), "{error}");
    }

    #[tokio::test]
    async fn boot_probe_preserves_http_response_readiness_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let html = r#"<script>globalThis["__DSH_BOOT__"] = {"entries":[{"url":"plugins/fixture/client.js"}]};</script>"#;
        for status in ["200 OK", "503 Service Unavailable"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let mut server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len(),
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result = client_probe_endpoints(port).await;
            let served = tokio::time::timeout(std::time::Duration::from_secs(1), &mut server).await;
            server.abort();
            served.expect("HTTP fixture did not complete").unwrap();
            if status == "200 OK" {
                let (endpoints, boot_html) = result.unwrap();
                assert_eq!(
                    endpoints,
                    vec![format!("http://127.0.0.1:{port}/plugins/fixture/client.js")]
                );
                assert_eq!(boot_html, html);
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    "HARNESS_NOT_READY: boot page returned 503 Service Unavailable"
                );
            }
        }
    }

    #[test]
    fn readiness_requires_every_client_module() {
        assert!(!all_client_modules_ready(1, 2));
        assert!(all_client_modules_ready(2, 2));
        assert!(!all_client_modules_ready(0, 0));
    }

    #[tokio::test]
    async fn client_bundle_probe_keeps_endpoint_order_and_runs_concurrently() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let delays = [
            std::time::Duration::from_millis(400),
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(150),
        ];
        let serial = delays.iter().sum::<std::time::Duration>();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = [
            "/plugins/slow/client.js",
            "/plugins/ready/client.js",
            "/plugins/markup/client.js",
        ];
        let server = tokio::spawn(async move {
            for _ in 0..paths.len() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]).to_string();
                let index = paths
                    .iter()
                    .position(|path| request.contains(*path))
                    .expect("request path is one of the probed endpoints");
                let body = if index == 1 {
                    "export const ready = true;"
                } else {
                    "<!doctype html><html></html>"
                };
                tokio::spawn(async move {
                    tokio::time::sleep(delays[index]).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len(),
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let endpoints = paths
            .iter()
            .map(|path| format!("http://127.0.0.1:{port}{path}"))
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let (ready, failures) = probe_client_bundles(&client, endpoints.clone()).await;
        let elapsed = started.elapsed();
        let served = tokio::time::timeout(std::time::Duration::from_secs(10), server).await;
        served
            .expect("bundle fixture did not serve every request")
            .unwrap();

        assert_eq!(ready, 1);
        assert_eq!(
            failures,
            vec![
                format!("{} returned 200 OK (not a plugin bundle)", endpoints[0]),
                format!("{} returned 200 OK (not a plugin bundle)", endpoints[2]),
            ]
        );
        assert!(
            elapsed < serial,
            "probe waited {elapsed:?} for three endpoints"
        );
    }
}
