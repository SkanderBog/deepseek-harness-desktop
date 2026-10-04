#[cfg(test)]
mod tests {
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn initialize_http(cache: &std::path::Path) -> tauri::Result<tauri::App<tauri::test::MockRuntime>> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = cache.to_string_lossy().into_owned();
        mock_builder()
            .plugin(tauri_plugin_http::init())
            .build(context)
    }

    #[test]
    fn public_asset_http_does_not_require_a_writable_cache_directory() {
        let scratch = tempfile::tempdir().unwrap();
        let cache = scratch.path().join("blocked-cache");
        std::fs::write(&cache, b"preserve this file").unwrap();
        let result = initialize_http(&cache);
        assert!(result.is_ok(), "HTTP initialization failed: {:?}", result.err());
        assert_eq!(std::fs::read(cache).unwrap(), b"preserve this file");
    }

    #[test]
    fn public_asset_http_does_not_create_a_cookie_store() {
        let scratch = tempfile::tempdir().unwrap();
        let cache = scratch.path().join("unused-cache");
        let _app = initialize_http(&cache).unwrap();
        assert!(!cache.exists(), "public asset HTTP created a cookie cache");
    }
}
