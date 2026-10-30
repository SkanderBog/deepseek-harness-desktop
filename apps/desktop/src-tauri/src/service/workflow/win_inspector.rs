use crate::AppHandle;

#[cfg(windows)]
mod imp {
    use crate::AppHandle;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::service::plugin::{parse_patch_layer, render_patch_layer};

    const PATCH_MARKER: &str = "dsh-win-terminal-inspector";

    const GIT_BASH_CANDIDATES: [&str; 4] = [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\usr\bin\bash.exe",
    ];

    fn profile_dir(app_handle: &AppHandle) -> PathBuf {
        crate::service::profile::profile_dir_of(
            app_handle,
            &crate::service::profile::active_profile(app_handle),
        )
    }

    fn write_file(path: &Path, content: &str) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create parent dir failed: {e}"))?;
        }
        fs::write(path, content).map_err(|e| format!("write {} failed: {e}", path.display()))
    }

    pub(super) fn apply_at(profile: &Path) -> Result<(), String> {
        let patch_path = profile.join("cordis.patch.yml");
        let existing = match fs::read_to_string(&patch_path) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        let doc = match parse_patch_list(&existing) {
            Ok(d) => d,
            Err(_) => return Ok(()),
        };
        let serde_yaml::Value::Sequence(seq) = doc else {
            return Ok(());
        };
        if !seq.iter().any(block_is_ours) {
            return Ok(());
        }

        let retained: Vec<serde_yaml::Value> =
            seq.into_iter().filter(|el| !block_is_ours(el)).collect();
        let out = render_patch_layer(&serde_yaml::Value::Sequence(retained))
            .map_err(|e| format!("PATCH_RENDER_FAILED: {e}"))?;
        write_file(&patch_path, &out).map_err(|e| format!("PATCH_PRUNE_FAILED: {e}"))
    }

    fn parse_patch_list(content: &str) -> Result<serde_yaml::Value, String> {
        if content.trim().is_empty() {
            return Ok(serde_yaml::Value::Sequence(Vec::new()));
        }
        let doc = parse_patch_layer(content).map_err(|e| format!("PATCH_PARSE_FAILED: {e}"))?;
        match &doc {
            serde_yaml::Value::Sequence(_) => Ok(doc),
            serde_yaml::Value::Null => Ok(serde_yaml::Value::Sequence(Vec::new())),
            _ => Err("PATCH_NOT_ARRAY: cordis.patch.yml must be a top-level array".to_string()),
        }
    }

    fn block_is_ours(el: &serde_yaml::Value) -> bool {
        serde_yaml::to_string(el)
            .map(|s| s.contains(PATCH_MARKER))
            .unwrap_or(false)
    }

    fn find_git_bash() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("DSH_GIT_BASH_PATH") {
            let candidate = PathBuf::from(p);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        GIT_BASH_CANDIDATES
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    }

    pub fn git_bash_bin_dirs() -> Vec<PathBuf> {
        let Some(bash) = find_git_bash() else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        if let Some(bin_dir) = bash.parent() {
            dirs.push(bin_dir.to_path_buf());
        }
        if let Some(usr_bin) = bash
            .parent()
            .and_then(Path::parent)
            .map(|git_root| git_root.join("usr").join("bin"))
            .filter(|p| p.is_dir())
        {
            dirs.push(usr_bin);
        }
        dirs
    }

    pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
        apply_at(&profile_dir(app_handle))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp_dir(tag: &str) -> PathBuf {
            std::env::temp_dir().join(format!("win-inspector-test-{}-{tag}", std::process::id()))
        }

        #[test]
        fn patch_prune_removes_only_our_insert_block() {
            let dir = temp_dir("i");
            std::fs::create_dir_all(&dir).unwrap();
            let patch = dir.join("cordis.patch.yml");
            std::fs::write(
                &patch,
                "# user comments\n- insert:\n    - id: win-terminal-inspector\n      name: dsh-win-terminal-inspector\n- id: some-row\n  config:\n    a: 1\n",
            )
            .unwrap();

            apply_at(&dir).unwrap();
            let out = std::fs::read_to_string(&patch).unwrap();
            assert!(!out.contains("win-terminal-inspector"));
            assert!(!out.contains("insert:"));
            assert!(out.contains("some-row"));

            apply_at(&dir).unwrap();
            let again = std::fs::read_to_string(&patch).unwrap();
            assert_eq!(out, again);

            std::fs::remove_dir_all(&dir).ok();
        }

        #[test]
        fn patch_prune_self_repairs_comment_only_remainder() {
            let dir = temp_dir("j");
            std::fs::create_dir_all(&dir).unwrap();
            let patch = dir.join("cordis.patch.yml");
            std::fs::write(
                &patch,
                "# Your patch layer for this dsh profile\n- insert:\n    - id: win-terminal-inspector\n      name: dsh-win-terminal-inspector\n",
            )
            .unwrap();

            apply_at(&dir).unwrap();
            let out = std::fs::read_to_string(&patch).unwrap();
            assert!(!out.contains("win-terminal-inspector"));
            assert!(out.contains("[]\n"));

            apply_at(&dir).unwrap();
            let again = std::fs::read_to_string(&patch).unwrap();
            assert_eq!(out, again);

            std::fs::remove_dir_all(&dir).ok();
        }

        #[test]
        fn patch_prune_leaves_foreign_patch_untouched() {
            let dir = temp_dir("k");
            std::fs::create_dir_all(&dir).unwrap();
            let patch = dir.join("cordis.patch.yml");
            let content = "- id: some-row\n  config:\n    a: 1\n";
            std::fs::write(&patch, content).unwrap();

            apply_at(&dir).unwrap();
            assert_eq!(std::fs::read_to_string(&patch).unwrap(), content);

            std::fs::remove_dir_all(&dir).ok();
        }

        #[test]
        fn patch_prune_keeps_js_expressions() {
            let dir = temp_dir("l");
            std::fs::create_dir_all(&dir).unwrap();
            let patch = dir.join("cordis.patch.yml");
            std::fs::write(
                &patch,
                "- insert:\n    - id: win-terminal-inspector\n      name: dsh-win-terminal-inspector\n- id: some-row\n  config:\n    token: !!js process.env.TOKEN\n",
            )
            .unwrap();

            apply_at(&dir).unwrap();
            assert_eq!(
                std::fs::read_to_string(&patch).unwrap(),
                "- id: some-row\n  config:\n    token: !!js process.env.TOKEN\n"
            );

            std::fs::remove_dir_all(&dir).ok();
        }

        #[test]
        fn git_bash_dirs_follow_finder() {
            match find_git_bash() {
                Some(bash) => {
                    let dirs = git_bash_bin_dirs();
                    assert!(dirs.contains(&bash.parent().unwrap().to_path_buf()));
                }
                None => assert!(git_bash_bin_dirs().is_empty()),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::AppHandle;

    pub fn apply(_app_handle: &AppHandle) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn apply_at(_profile: &std::path::Path) -> Result<(), String> {
        Ok(())
    }

    pub fn git_bash_bin_dirs() -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}

pub fn apply(app_handle: &AppHandle) -> Result<(), String> {
    imp::apply(app_handle)
}

pub(crate) fn apply_at(profile: &std::path::Path) -> Result<(), String> {
    imp::apply_at(profile)
}

pub fn git_bash_bin_dirs() -> Vec<std::path::PathBuf> {
    imp::git_bash_bin_dirs()
}
