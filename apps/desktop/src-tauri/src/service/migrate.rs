//! checkCompatibility 阶段抛 `ERR_PNPM_UNEXPECTED_VIRTUAL_STORE`（见 issue #103）。

use std::fs;
use std::path::Path;

pub(crate) fn purge_carried_pnpm_metadata(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if entry.file_name() == "node_modules" {
            let modules_yaml = path.join(".modules.yaml");
            if modules_yaml.is_file() {
                match fs::remove_file(&modules_yaml) {
                    Ok(()) => log::info!(
                        "purged carried pnpm modules metadata: {}",
                        modules_yaml.display()
                    ),
                    Err(e) => log::warn!("purge {} failed: {e}", modules_yaml.display()),
                }
            }
        } else {
            purge_carried_pnpm_metadata(&path);
        }
    }
}

pub fn heal_stale_pnpm_metadata(dsh_home: &Path) -> Result<(), String> {
    let profiles = dsh_home.join("profiles");
    let Ok(entries) = fs::read_dir(&profiles) else {
        return Ok(()); // 全新安装 / 无 profiles 目录 → 无可修对象
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            purge_if_stale_modules_metadata(&entry.path().join("node_modules"), dsh_home);
        }
    }
    Ok(())
}

fn purge_if_stale_modules_metadata(node_modules: &Path, dsh_home: &Path) {
    let modules_yaml = node_modules.join(".modules.yaml");
    if !modules_yaml.is_file() {
        return;
    }
    let Ok(content) = fs::read_to_string(&modules_yaml) else {
        return;
    };
    let Ok(doc): Result<serde_yaml::Value, _> = serde_yaml::from_str(&content) else {
        return;
    };
    let stale = doc.as_mapping().is_some_and(|mapping| {
        ["virtualStoreDir", "storeDir"].iter().any(|key| {
            mapping
                .get(serde_yaml::Value::String((*key).into()))
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(|value| {
                    let p = Path::new(value);
                    p.is_absolute() && !p.starts_with(dsh_home) && !p.exists()
                })
        })
    });
    if stale {
        match fs::remove_file(&modules_yaml) {
            Ok(()) => log::info!(
                "purged stale pnpm modules metadata (old DSH_HOME paths): {}",
                modules_yaml.display()
            ),
            Err(e) => log::warn!("purge {} failed: {e}", modules_yaml.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-migrate-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    // 启动自愈：清理指向旧位置的 pnpm 元数据（老搬迁残留，issue #103）

    #[test]
    fn heal_purges_stale_absolute_virtual_store_paths() {
        let home = temp_dir("heal-home");
        let node_modules = home.join("profiles/web/node_modules");
        let legacy_home = temp_dir("heal-legacy-home");
        let legacy_vsd = legacy_home.join("profiles/web/node_modules/.pnpm");
        write(
            &node_modules.join(".modules.yaml"),
            &format!(
                "lockfileVersion: '9.0'\nstoreDir: {}\nvirtualStoreDir: {}\n",
                legacy_home.join("pnpm/store/v10").display(),
                legacy_vsd.display()
            ),
        );
        fs::create_dir_all(home.join("profiles/web/node_modules/.pnpm")).unwrap();

        heal_stale_pnpm_metadata(&home).unwrap();

        assert!(
            !node_modules.join(".modules.yaml").exists(),
            "stale pnpm metadata pointing at removed old DSH_HOME must be purged"
        );
    }

    #[test]
    fn heal_keeps_consistent_or_relative_modules_metadata() {
        let home = temp_dir("heal-keep");
        let node_modules = home.join("profiles/web/node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        let store = temp_dir("heal-keep-store");
        write(
            &node_modules.join(".modules.yaml"),
            &format!(
                "lockfileVersion: '9.0'\nstoreDir: {}\nvirtualStoreDir: node_modules/.pnpm\n",
                store.display()
            ),
        );

        heal_stale_pnpm_metadata(&home).unwrap();
        assert!(
            node_modules.join(".modules.yaml").is_file(),
            "valid modules metadata must be kept"
        );
    }

    #[test]
    fn heal_keeps_absolute_paths_living_under_current_home() {
        let home = temp_dir("heal-under-home");
        let node_modules = home.join("profiles/web/node_modules");
        let vsd = home.join("profiles/web/node_modules/.pnpm");
        fs::create_dir_all(&vsd).unwrap();
        write(
            &node_modules.join(".modules.yaml"),
            &format!(
                "lockfileVersion: '9.0'\nvirtualStoreDir: {}\n",
                vsd.display()
            ),
        );

        heal_stale_pnpm_metadata(&home).unwrap();
        assert!(
            node_modules.join(".modules.yaml").is_file(),
            "absolute path under current DSH_HOME is consistent, must be kept"
        );
    }

    #[test]
    fn heal_missing_profiles_dir_is_noop() {
        let home = temp_dir("heal-empty");
        heal_stale_pnpm_metadata(&home).unwrap();
        assert!(!home.join("profiles").exists());
    }
}
