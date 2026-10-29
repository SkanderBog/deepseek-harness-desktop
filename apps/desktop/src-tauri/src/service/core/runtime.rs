//! 阶段退出、前端只看到 `HARNESS_NOT_OWNED`（issue #441）。这里在 spawn 之前探测，

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::fs::FileTypeExt;

use super::source::CoreSource;
use crate::{AppHandle};

const CORE_PACKAGE_SCOPE: &str = "@deepseek-ai";
const CORE_PACKAGE_PREFIX: &str = "dsh";
const NATIVE_REPAIR_TIMEOUT: Duration = Duration::from_secs(120);
const NATIVE_REBUILD_TIMEOUT: Duration = Duration::from_secs(300);
const NATIVE_PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const NODE_PROBE_SCRIPT: &str = "process.stdout.write(process.platform + ':' + process.arch)";
const NATIVE_IMPORT_SCRIPT: &str = "await import('sharp'); await import('koffi')";
const NATIVE_PROBE_MARKER: &str = "__DSH_NATIVE_PROBE__";
/// 已核验通过的原生依赖结论戳（issue #766）。
const NATIVE_PROBE_STAMP_FILE: &str = "core-native-probe.stamp.json";
const NATIVE_PROBE_STAMP_MAX_ENTRIES: usize = 8192;
const NATIVE_PROBE_SCRIPT: &str = r#"
const { createRequire } = await import('node:module');
const { existsSync, readdirSync } = await import('node:fs');
const { join } = await import('node:path');
const root = process.cwd();
const loader = createRequire(join(root, 'dsh-native-probe.cjs'));
const failures = [];
// 扫描中途抛错时置位：此时 failures 可能只是「还没扫到」的空清单，绝不能当成
// 「原生依赖都正常」报上去（见脚本末尾的标记行输出条件）。
let incomplete = false;
const describe = (error) => String((error && error.message) || error);
const isAbi = (text) => text.includes('NODE_MODULE_VERSION');
const attempt = (name, target) => {
  try { loader(target); return null; } catch (error) { return { name, message: describe(error) }; }
};
for (const name of ['sharp', 'koffi']) {
  const failure = attempt(name, name);
  if (failure) failures.push({ kind: 'import', name, message: failure.message, abi: isAbi(failure.message) });
}
try {
  const nodeModules = join(root, 'node_modules');
  const outputs = ['build/Release', 'build/Debug', 'prebuilds', 'prebuilt'];
  const candidates = [];
  const collect = (dir, name) => {
    if (existsSync(join(dir, 'binding.gyp')) || outputs.some((relative) => existsSync(join(dir, relative)))) {
      candidates.push({ dir, name });
    }
  };
  if (existsSync(nodeModules)) {
    // 目录链接（pnpm 布局 / 桌面端为插件建立的 junction）在 withFileTypes 下是
    // symbolic link 而非 directory，必须一并纳入，否则会漏掉链接形式的原生包。
    const isPackageDir = (entry) => entry.isDirectory() || entry.isSymbolicLink();
    for (const entry of readdirSync(nodeModules, { withFileTypes: true })) {
      if (!isPackageDir(entry)) continue;
      if (entry.name === '.bin' || entry.name === '.pnpm') continue;
      if (entry.name.startsWith('@')) {
        const scope = join(nodeModules, entry.name);
        for (const inner of readdirSync(scope, { withFileTypes: true })) {
          if (isPackageDir(inner)) collect(join(scope, inner.name), entry.name + '/' + inner.name);
        }
        continue;
      }
      collect(join(nodeModules, entry.name), entry.name);
    }
  }
  for (const pkg of candidates) {
    let failure = attempt(pkg.name, pkg.name);
    if (failure) {
      // 包入口不可 require（纯 ESM / 无 main）时退回直接加载 node-gyp 产物；
      // 只用来"洗清"失败，绝不覆盖包入口给出的错误信息（后者更完整，且
      // 直接加载 .node 可能报成"不是有效的 Win32 应用程序"而掩盖 ABI 差异）。
      const release = join(pkg.dir, 'build', 'Release');
      if (existsSync(release)) {
        const addon = readdirSync(release).find((file) => file.endsWith('.node'));
        if (addon && attempt(pkg.name, join(release, addon)) === null) failure = null;
      }
    }
    if (failure) failures.push({ kind: 'addon', name: pkg.name, message: failure.message, abi: isAbi(failure.message) });
  }
} catch (error) {
  process.stderr.write('native addon scan failed: ' + describe(error) + '\n');
  incomplete = true;
}
// 扫描没做完就不输出标记行：Rust 侧据此拿到「结论未知」（进而回退到 sharp/koffi
// 探测、不写结论戳），而不是一份「没有失败」的空清单被误判成 Ready。
if (!incomplete) process.stdout.write('__DSH_NATIVE_PROBE__' + JSON.stringify(failures) + '\n');
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeTarget {
    platform: String,
    arch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct NativeProbeFailure {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    abi: bool,
}

impl NativeProbeFailure {
    fn is_platform_import(&self) -> bool {
        self.kind == "import"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeProbe {
    Ready,
    Failed(Vec<NativeProbeFailure>),
    Unknown(String),
}

impl NativeProbe {
    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    fn failures(&self) -> &[NativeProbeFailure] {
        match self {
            Self::Failed(failures) => failures,
            _ => &[],
        }
    }

    fn has_abi_mismatch(&self) -> bool {
        self.failures().iter().any(|failure| failure.abi)
    }

    fn has_platform_import_failure(&self) -> bool {
        self.failures()
            .iter()
            .any(NativeProbeFailure::is_platform_import)
    }

    fn abi_packages(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for failure in self.failures().iter().filter(|failure| failure.abi) {
            if !names.contains(&failure.name) {
                names.push(failure.name.clone());
            }
        }
        names
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimePreparation {
    Ready,
    Defer,
}

pub(crate) struct RuntimeRequiredPlugin<'a> {
    pub name: &'a str,
    pub source: Option<PathBuf>,
    pub version: Option<&'a crate::config::manifest::PluginVersion>,
}

pub(crate) struct RuntimePreparationPaths<'a> {
    pub source: CoreSource,
    pub base: &'a Path,
    pub core_root: &'a Path,
    pub node: &'a Path,
    pub profile: &'a Path,
    pub profiles: &'a Path,
    pub resources: Option<&'a Path>,
    pub core_version: Option<&'a str>,
    pub required_plugins: Option<&'a [RuntimeRequiredPlugin<'a>]>,
}

pub(crate) fn prepare_active_runtime_at(
    paths: &RuntimePreparationPaths<'_>,
) -> Result<RuntimePreparation, String> {
    if paths.source != CoreSource::App {
        return Ok(RuntimePreparation::Defer);
    }
    if !paths.core_root.is_dir() {
        return Err(format!(
            "CORE_RUNTIME_NOT_FOUND: bundled core directory is missing: {}",
            paths.core_root.display()
        ));
    }
    if !paths.node.is_file() {
        return Err(format!(
            "CORE_RUNTIME_NODE_NOT_FOUND: Node.js runtime is missing: {}",
            paths.node.display()
        ));
    }
    let linked = link_required_plugins_at(paths)?;
    let pruned = match prune_stale_core_packages_at(paths.profile, paths.core_root) {
        Ok(ready) => ready,
        Err(error) => {
            log::warn!("{error}");
            false
        }
    };
    if !linked || !pruned || paths.core_version.is_none_or(str::is_empty) {
        return Ok(RuntimePreparation::Defer);
    }
    let key = native_probe_stamp_key_at(paths.node, paths.core_root, paths.core_version);
    if let Some(key) = key.as_deref() {
        if let Some((stored, platform, arch)) = read_native_probe_stamp_at(paths.base) {
            if stored == key {
                log::debug!(
                    "Bundled core native dependencies are ready for {platform}:{arch} (cached)"
                );
                return Ok(RuntimePreparation::Ready);
            }
        }
    }
    Ok(RuntimePreparation::Defer)
}

pub(crate) async fn prepare_active_runtime(app_handle: &AppHandle) -> Result<(), String> {
    if crate::service::core::active_source(app_handle) != CoreSource::App {
        log::debug!("Skipping core runtime repair for user-owned local core");
        return Ok(());
    }

    let core_root = crate::config::get_dsh_install_path(app_handle);
    let node = crate::config::get_node_binary_path(app_handle);
    let base = crate::config::get_base_dir(app_handle);
    let profile = crate::service::plugin::profile_dir(app_handle);
    let profiles = crate::config::get_dsh_data_path(app_handle).join("profiles");
    let resources = crate::config::manifest::resource_root(app_handle);
    let core_version = crate::service::core::active_version(app_handle);
    let presets = crate::service::plugin::load_presets(app_handle);
    let required_plugins: Vec<_> = presets
        .iter()
        .filter(|preset| preset.internal)
        .map(|preset| RuntimeRequiredPlugin {
            name: crate::service::plugin::installed_name(preset),
            source: crate::service::plugin::bundled_plugin_dir(app_handle, &preset.id),
            version: preset.version.as_ref(),
        })
        .collect();
    let manifest_known = crate::config::manifest::with_manifest(app_handle, |_| ()).is_some();
    let paths = RuntimePreparationPaths {
        source: CoreSource::App,
        base: &base,
        core_root: &core_root,
        node: &node,
        profile: &profile,
        profiles: &profiles,
        resources: resources.as_deref(),
        core_version: core_version.as_deref(),
        required_plugins: manifest_known.then_some(required_plugins.as_slice()),
    };
    if prepare_active_runtime_at(&paths)? == RuntimePreparation::Ready {
        return Ok(());
    }
    let stamp_key = native_probe_stamp_key_at(&node, &core_root, core_version.as_deref());

    let target = detect_node_target(&node, &core_root).await?;
    let mut probe = probe_native_modules(&node, &core_root).await;
    if probe.is_ready() {
        log::debug!(
            "Bundled core native dependencies are ready for {}:{}",
            target.platform,
            target.arch
        );
        if let Some(key) = stamp_key.as_deref() {
            write_native_probe_stamp_at(&base, key, &target);
        }
        return Ok(());
    }
    if let NativeProbe::Unknown(reason) = probe.clone() {
        log::warn!("CORE_NATIVE_PROBE_UNKNOWN: {reason}");
        if native_imports_ready(&node, &core_root).await {
            return Ok(());
        }
        probe = NativeProbe::Failed(vec![NativeProbeFailure {
            kind: "import".into(),
            name: "sharp/koffi".into(),
            message: reason,
            abi: false,
        }]);
    }

    if probe.has_platform_import_failure() {
        let packages = native_package_plan(&core_root, &target);
        if packages.is_empty() {
            return Err(format!(
                "CORE_NATIVE_DEPENDENCY_UNRESOLVED: no optional package version matches {}:{} in {}",
                target.platform,
                target.arch,
                core_root.display()
            ));
        }
        log::warn!(
            "CORE_NATIVE_DEPENDENCY_REPAIR: importing sharp/koffi failed for {}:{}, installing {:?}",
            target.platform,
            target.arch,
            packages
        );
        install_native_packages(&core_root, &target, &packages).await?;
        let Some(next) = reprobe_after_repair(&node, &core_root).await else {
            return Ok(());
        };
        probe = next;
    }

    // 2) ABI 不匹配（issue #441）：本地 node 与核心原生模块的构建 ABI 不同。核心由
    let abi_packages = probe.abi_packages();
    if !abi_packages.is_empty() {
        log::warn!(
            "CORE_NATIVE_ABI_MISMATCH: rebuilding {:?} for {} in {}",
            abi_packages,
            node.display(),
            core_root.display()
        );
        match rebuild_native_packages(&core_root, &node, &abi_packages).await {
            Ok(()) => {
                let Some(next) = reprobe_after_repair(&node, &core_root).await else {
                    log::info!(
                        "CORE_NATIVE_REBUILD: rebuilt {:?} for {}",
                        abi_packages,
                        node.display()
                    );
                    return Ok(());
                };
                probe = next;
            }
            Err(error) => log::warn!("CORE_NATIVE_REBUILD_FAILED: {error}"),
        }
    }

    Err(native_failure_diagnostic(
        probe.failures(),
        &node,
        &core_root,
    ))
}

fn prune_stale_core_packages_at(profile: &Path, core_root: &Path) -> Result<bool, String> {
    let declared = std::fs::read_to_string(profile.join("package.json"))
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<crate::service::plugin::ProfilePackageJson>(&raw).ok()
        });
    let Some(manifest) = declared else {
        log::warn!(
            "CORE_PLUGIN_STALE_CORE_DECLARED_UNKNOWN: {} is unreadable, skipping cleanup",
            profile.display()
        );
        return Ok(false);
    };
    let mut declared: HashSet<_> = manifest.dependencies.into_keys().collect();
    if let Some(profile) = manifest.dsh.and_then(|dsh| dsh.profile) {
        declared.extend(profile.bundles);
    }
    prune_stale_core_entries(profile, &core_root.join("node_modules"), &declared)
}

fn prune_stale_core_entries(
    profile_root: &Path,
    anchor_node_modules: &Path,
    declared: &HashSet<String>,
) -> Result<bool, String> {
    let profile_node_modules = profile_root.join("node_modules");
    let scope = profile_node_modules.join(CORE_PACKAGE_SCOPE);
    if std::fs::symlink_metadata(&scope)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(true);
    }
    let Some(scope_root) = containment_root(&scope, profile_root) else {
        return Ok(false);
    };
    let mut ready = true;
    let entries = match std::fs::read_dir(&scope) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => {
            return Err(format!(
                "CORE_PLUGIN_STALE_CORE_SCAN_FAILED: {}: {e}",
                scope.display()
            ))
        }
    };

    for entry in entries {
        let entry = entry.map_err(|e| {
            format!(
                "CORE_PLUGIN_STALE_CORE_SCAN_FAILED: {}: {e}",
                scope.display()
            )
        })?;
        let name = format!(
            "{CORE_PACKAGE_SCOPE}/{}",
            entry.file_name().to_string_lossy()
        );
        if !is_safe_package_name(&name) || declared.contains(&name) {
            continue;
        }
        if !is_core_family_package(&name) {
            continue;
        }
        let path = entry.path();
        if !entry_is_contained(&path, &scope_root) {
            log::warn!(
                "CORE_PLUGIN_STALE_CORE_OUT_OF_SCOPE: {} escapes {}, skipping",
                path.display(),
                scope.display()
            );
            ready = false;
            continue;
        }
        let Some(profile_version) = read_package_version(&path.join("package.json")) else {
            ready = false;
            continue;
        };
        let Some(anchor_version) =
            read_package_version(&anchor_node_modules.join(&name).join("package.json"))
        else {
            ready = false;
            continue;
        };
        if profile_version == anchor_version {
            continue;
        }
        if let Err(e) = remove_core_package_residue(&path) {
            log::warn!("{e}");
            ready = false;
            continue;
        }
        log::info!(
            "CORE_PLUGIN_STALE_CORE_PRUNED: {name} {profile_version} (profile) != {anchor_version} (anchor), removed"
        );
    }
    Ok(ready)
}

fn containment_root(scope: &Path, profile_root: &Path) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(scope).ok()?;
    let file_type = metadata.file_type();
    #[cfg(windows)]
    let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
    #[cfg(not(windows))]
    let is_link = file_type.is_symlink();
    if is_link {
        log::warn!(
            "CORE_PLUGIN_STALE_CORE_SCOPE_REDIRECTED: {} is a link, skipping cleanup",
            scope.display()
        );
        return None;
    }

    let root = std::fs::canonicalize(profile_root).ok()?;
    let real = std::fs::canonicalize(scope).ok()?;
    if !real.starts_with(&root) {
        log::warn!(
            "CORE_PLUGIN_STALE_CORE_SCOPE_ESCAPED: {} resolves to {}, skipping cleanup",
            scope.display(),
            real.display()
        );
        return None;
    }
    Some(real)
}

fn entry_is_contained(path: &Path, scope_root: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let file_type = metadata.file_type();
    #[cfg(windows)]
    if file_type.is_symlink() || file_type.is_symlink_dir() {
        return true;
    }
    #[cfg(not(windows))]
    if file_type.is_symlink() {
        return true;
    }
    match std::fs::canonicalize(path) {
        Ok(real) => real.starts_with(scope_root),
        Err(_) => false,
    }
}

fn remove_core_package_residue(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        format!(
            "CORE_PLUGIN_STALE_CORE_REMOVE_FAILED: {}: {e}",
            path.display()
        )
    })?;
    let file_type = metadata.file_type();
    #[cfg(windows)]
    let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
    #[cfg(not(windows))]
    let is_link = file_type.is_symlink();
    if is_link {
        return remove_link_only(path);
    }
    let result = if file_type.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|e| {
        format!(
            "CORE_PLUGIN_STALE_CORE_REMOVE_FAILED: {}: {e}",
            path.display()
        )
    })
}

/// 共享核心只保留内置插件入口，并移除旧版投射的档案插件链接（issue #993）。
fn link_required_plugins_at(paths: &RuntimePreparationPaths<'_>) -> Result<bool, String> {
    let core_node_modules = paths.core_root.join("node_modules");
    std::fs::create_dir_all(&core_node_modules).map_err(|e| {
        format!(
            "CORE_PLUGIN_NODE_MODULES_CREATE_FAILED: {}: {e}",
            core_node_modules.display()
        )
    })?;

    let internal: Vec<_> = paths
        .required_plugins
        .unwrap_or_default()
        .iter()
        .filter(|plugin| {
            !plugin
                .version
                .is_some_and(|version| version.unsupported_on(paths.core_version))
        })
        .collect();
    let internal_ids = internal
        .iter()
        .map(|plugin| plugin.name.to_string())
        .collect();
    let mut ready = paths.required_plugins.is_some();
    for plugin in internal {
        let source = plugin.source.as_deref().and_then(|source| {
            if source.is_absolute() {
                Some(source.to_path_buf())
            } else {
                paths.resources.map(|root| root.join(source))
            }
        });
        let Some(source) = source else {
            log::warn!(
                "CORE_PLUGIN_BUNDLE_MISSING: bundled source unavailable for {}, skipping core link",
                plugin.name
            );
            ready = false;
            continue;
        };
        ready &= ensure_package_link(plugin.name, &source, &core_node_modules)?;
    }

    Ok(remove_profile_plugin_links(&core_node_modules, paths.profiles, &internal_ids)? && ready)
}

fn remove_profile_plugin_links(
    core_modules: &Path,
    profiles: &Path,
    internal_ids: &HashSet<String>,
) -> Result<bool, String> {
    let entries = match std::fs::read_dir(profiles) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(format!("CORE_PLUGIN_PROFILES_READ_FAILED: {error}")),
    };
    let mut ready = true;
    for entry in entries {
        let entry = entry.map_err(|error| format!("CORE_PLUGIN_PROFILE_READ_FAILED: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("CORE_PLUGIN_PROFILE_STAT_FAILED: {error}"))?
            .is_dir()
            || entry.file_name() == "node_modules"
        {
            continue;
        }
        let manifest_path = entry.path().join("package.json");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
            ready = false;
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw) else {
            ready = false;
            continue;
        };
        let mut names = HashSet::new();
        if let Some(dependencies) = manifest
            .get("dependencies")
            .and_then(|value| value.as_object())
        {
            names.extend(dependencies.keys().map(String::as_str));
        }
        if let Some(bundles) = manifest
            .pointer("/dsh/profile/bundles")
            .and_then(|value| value.as_array())
        {
            names.extend(bundles.iter().filter_map(|value| value.as_str()));
        }
        for name in names {
            if internal_ids.contains(name) || !is_safe_package_name(name) {
                continue;
            }
            let destination = core_modules.join(name);
            let metadata = match std::fs::symlink_metadata(&destination) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    ready = false;
                    continue;
                }
            };
            let file_type = metadata.file_type();
            #[cfg(windows)]
            let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
            #[cfg(not(windows))]
            let is_link = file_type.is_symlink();
            if !is_link {
                continue;
            }
            let source = entry.path().join("node_modules").join(name);
            match (source.canonicalize(), destination.canonicalize()) {
                (Ok(source), Ok(target)) if source == target => {
                    ensure_non_link_directory(destination.parent().unwrap(), core_modules)?;
                    remove_link_only(&destination)?;
                }
                (Ok(_), Ok(_)) => {}
                _ => ready = false,
            }
        }
    }
    Ok(ready)
}

fn ensure_package_link(name: &str, source: &Path, node_modules: &Path) -> Result<bool, String> {
    if !is_safe_package_name(name) {
        return Err(format!("CORE_PLUGIN_NAME_INVALID: {name}"));
    }
    let source = source.canonicalize().map_err(|e| {
        format!(
            "CORE_PLUGIN_SOURCE_INVALID: {} ({name}): {e}",
            source.display()
        )
    })?;
    if !source.is_dir() || !source.join("package.json").is_file() {
        return Err(format!(
            "CORE_PLUGIN_SOURCE_INVALID: {name} is not a package directory: {}",
            source.display()
        ));
    }
    let package_name = read_package_name(&source.join("package.json"))?;
    if package_name.as_deref() != Some(name) {
        return Err(format!(
            "CORE_PLUGIN_SOURCE_NAME_MISMATCH: requested {name}, source declares {}",
            package_name.unwrap_or_else(|| "<missing>".to_string())
        ));
    }

    let destination = node_modules.join(name);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("CORE_PLUGIN_DESTINATION_INVALID: {name}"))?;
    ensure_non_link_directory(parent, node_modules)?;

    let existing = match std::fs::symlink_metadata(&destination) {
        Ok(metadata) => Some(metadata),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(format!(
                "CORE_PLUGIN_DESTINATION_STAT_FAILED: {destination:?}: {e}"
            ))
        }
    };
    if let Some(metadata) = existing {
        let file_type = metadata.file_type();
        #[cfg(windows)]
        let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
        #[cfg(not(windows))]
        let is_link = file_type.is_symlink();
        if !is_link {
            log::debug!(
                "CORE_PLUGIN_ENTRY_OCCUPIED: keeping core-owned entry {}",
                destination.display()
            );
            return Ok(file_type.is_dir()
                && read_package_name(&destination.join("package.json"))
                    .is_ok_and(|package_name| package_name.as_deref() == Some(name)));
        }
        let matches = std::fs::read_link(&destination)
            .ok()
            .map(|target| {
                let resolved = if target.is_absolute() {
                    target
                } else {
                    destination.parent().unwrap_or(Path::new(".")).join(target)
                };
                resolved.canonicalize().ok().as_deref() == Some(source.as_path())
            })
            .unwrap_or(false);
        if matches {
            return Ok(true);
        }
        remove_link_only(&destination)?;
    }

    create_directory_link(&source, &destination).map_err(|e| {
        format!(
            "CORE_PLUGIN_LINK_FAILED: {} -> {}: {e}",
            source.display(),
            destination.display()
        )
    })?;
    log::info!(
        "CORE_PLUGIN_LINKED: {} -> {}",
        destination.display(),
        source.display()
    );
    Ok(destination.canonicalize().ok().as_deref() == Some(source.as_path()))
}

fn ensure_non_link_directory(path: &Path, root: &Path) -> Result<(), String> {
    if !path.starts_with(root) {
        return Err(format!(
            "CORE_PLUGIN_DESTINATION_ESCAPE: {}",
            path.display()
        ));
    }
    let relative = path.strip_prefix(root).unwrap_or(Path::new("."));
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if {
                    let file_type = metadata.file_type();
                    #[cfg(windows)]
                    let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
                    #[cfg(not(windows))]
                    let is_link = file_type.is_symlink();
                    is_link
                } =>
            {
                return Err(format!(
                    "CORE_PLUGIN_DESTINATION_PARENT_LINK: {}",
                    current.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "CORE_PLUGIN_DESTINATION_PARENT_INVALID: {}",
                    current.display()
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|e| {
                    format!(
                        "CORE_PLUGIN_DESTINATION_PARENT_CREATE_FAILED: {}: {e}",
                        current.display()
                    )
                })?;
            }
            Err(e) => {
                return Err(format!(
                    "CORE_PLUGIN_DESTINATION_PARENT_STAT_FAILED: {}: {e}",
                    current.display()
                ))
            }
        }
    }
    Ok(())
}

fn remove_link_only(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let result = std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path));
    #[cfg(not(windows))]
    let result = std::fs::remove_file(path);
    result.map_err(|e| format!("CORE_PLUGIN_LINK_REMOVE_FAILED: {}: {e}", path.display()))
}

#[cfg(unix)]
pub(crate) fn create_directory_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
pub(crate) fn create_directory_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    match std::os::windows::fs::symlink_dir(source, destination) {
        Ok(()) => Ok(()),
        Err(e) if is_privilege_error(&e) => {
            log::warn!(
                "CORE_PLUGIN_SYMLINK_FALLBACK: symlink_dir failed ({}), creating junction instead",
                e
            );
            create_directory_junction(source, destination)
        }
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
fn is_privilege_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(1314 | 5))
}

#[cfg(windows)]
fn create_directory_junction(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let source = source.canonicalize()?;
    let simplified = dunce::simplified(&source);
    let print_name = simplified.to_string_lossy().replace('/', "\\");
    let substitute_name = if print_name.starts_with("\\\\") {
        format!(r"\??\UNC\{}", print_name.trim_start_matches('\\'))
    } else {
        format!(r"\??\{}", print_name)
    };

    let substitute_wide: Vec<u16> = substitute_name.encode_utf16().collect();
    let print_wide: Vec<u16> = print_name.encode_utf16().collect();
    let substitute_bytes = substitute_wide.len() * 2;
    let print_bytes = print_wide.len() * 2;
    const REPARSE_GAP: usize = 2;
    const REPARSE_TRAILING: usize = 2;
    let data_length = 8 + substitute_bytes + REPARSE_GAP + print_bytes + REPARSE_TRAILING;
    let total = 16 + substitute_bytes + REPARSE_GAP + print_bytes + REPARSE_TRAILING;

    let mut buffer = vec![0u8; total];
    buffer[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer[4..6].copy_from_slice(&(data_length as u16).to_le_bytes());
    buffer[8..10].copy_from_slice(&0u16.to_le_bytes());
    buffer[10..12].copy_from_slice(&(substitute_bytes as u16).to_le_bytes());
    buffer[12..14].copy_from_slice(&((substitute_bytes + REPARSE_GAP) as u16).to_le_bytes());
    buffer[14..16].copy_from_slice(&(print_bytes as u16).to_le_bytes());
    let mut cursor = 16usize;
    for unit in substitute_wide.iter().copied() {
        buffer[cursor..cursor + 2].copy_from_slice(&unit.to_le_bytes());
        cursor += 2;
    }
    cursor += REPARSE_GAP;
    for unit in print_wide.iter().copied() {
        buffer[cursor..cursor + 2].copy_from_slice(&unit.to_le_bytes());
        cursor += 2;
    }

    let created_dir = if !destination.is_dir() {
        std::fs::create_dir_all(destination)?;
        true
    } else {
        false
    };
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            destination_wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        if created_dir {
            let _ = std::fs::remove_dir(destination);
        }
        return Err(std::io::Error::last_os_error());
    }
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr() as *const core::ffi::c_void,
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        if created_dir {
            let _ = std::fs::remove_dir(destination);
        }
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn is_safe_package_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() == 1 {
        return valid_package_component(parts[0]);
    }
    parts.len() == 2
        && parts[0].starts_with('@')
        && valid_package_component(parts[0])
        && valid_package_component(parts[1])
}

fn is_core_family_package(name: &str) -> bool {
    let Some(package) = name
        .strip_prefix(CORE_PACKAGE_SCOPE)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return false;
    };
    package == CORE_PACKAGE_PREFIX
        || package
            .strip_prefix(CORE_PACKAGE_PREFIX)
            .is_some_and(|suffix| suffix.starts_with('-'))
}

fn valid_package_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value != "@"
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-@".contains(&byte))
}

fn read_package_name(path: &Path) -> Result<Option<String>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "CORE_PLUGIN_PACKAGE_MANIFEST_READ_FAILED: {}: {e}",
            path.display()
        )
    })?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| {
        format!(
            "CORE_PLUGIN_PACKAGE_MANIFEST_INVALID: {}: {e}",
            path.display()
        )
    })?;
    Ok(value
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned))
}

fn read_package_version(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    value
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn native_probe_stamp_path_at(base: &Path) -> PathBuf {
    base.join(NATIVE_PROBE_STAMP_FILE)
}

fn native_probe_stamp_key_at(
    node: &Path,
    core_root: &Path,
    core_version: Option<&str>,
) -> Option<String> {
    use std::fmt::Write as _;

    let mut key = String::new();
    let _ = write!(key, "node={}", node.display());
    if let Ok(meta) = std::fs::metadata(node) {
        let _ = write!(key, ":{}", meta.len());
        let _ = write!(key, ":{}", file_modified_nanos(&meta));
    }
    let _ = write!(
        key,
        "\ncore={}\nversion={}",
        core_root.display(),
        core_version.unwrap_or_default()
    );

    let modules = probe_stamp_modules_digest(&core_root.join("node_modules"))?;
    let _ = write!(key, "\nmodules={modules}");
    Some(probe_stamp_digest(&key))
}

fn probe_stamp_modules_digest(modules: &Path) -> Option<String> {
    let mut entries: Vec<String> = Vec::new();
    collect_probe_stamp_entries(modules, &mut entries, NATIVE_PROBE_STAMP_MAX_ENTRIES)?;
    entries.sort_unstable();
    Some(probe_stamp_digest(&entries.join("\n")))
}

fn probe_stamp_digest(key: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(key.as_bytes()))
}

const NATIVE_ARTIFACT_DIRS: [&str; 4] = ["build/Release", "build/Debug", "prebuilds", "prebuilt"];

fn collect_probe_stamp_entries(dir: &Path, out: &mut Vec<String>, cap: usize) -> Option<()> {
    let reader = std::fs::read_dir(dir).ok()?;
    for entry in reader {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == ".bin" {
            continue;
        }
        push_probe_stamp_entry(&entry.path(), name, out, cap)?;
        if !name.starts_with('@') {
            continue;
        }
        let scope = std::fs::read_dir(entry.path()).ok()?;
        for inner in scope {
            let inner = inner.ok()?;
            let inner_name = inner.file_name();
            let Some(inner_name) = inner_name.to_str() else {
                continue;
            };
            push_probe_stamp_entry(&inner.path(), &format!("{name}/{inner_name}"), out, cap)?;
        }
    }
    Some(())
}

fn push_probe_stamp_entry(
    dir: &Path,
    label: &str,
    out: &mut Vec<String>,
    cap: usize,
) -> Option<()> {
    push_probe_stamp_item(out, format!("{label}={}", path_modified_nanos(dir)), cap)?;
    for relative in NATIVE_ARTIFACT_DIRS {
        let artifact = dir.join(relative);
        if !artifact.is_dir() {
            continue;
        }
        push_probe_stamp_item(
            out,
            format!("{label}/{relative}={}", path_modified_nanos(&artifact)),
            cap,
        )?;
        push_probe_stamp_artifacts(&artifact, &format!("{label}/{relative}"), out, cap)?;
    }
    Some(())
}

fn push_probe_stamp_item(out: &mut Vec<String>, item: String, cap: usize) -> Option<()> {
    out.push(item);
    if out.len() > cap {
        log::warn!("core native probe stamp skipped: more than {cap} entries");
        return None;
    }
    Some(())
}

fn push_probe_stamp_artifacts(
    dir: &Path,
    label: &str,
    out: &mut Vec<String>,
    cap: usize,
) -> Option<()> {
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = entry.path();
        let label = format!("{label}/{name}");
        let file_type = std::fs::symlink_metadata(&path).ok()?.file_type();
        #[cfg(windows)]
        let is_link = file_type.is_symlink() || file_type.is_symlink_dir();
        #[cfg(not(windows))]
        let is_link = file_type.is_symlink();
        if is_link && path.is_dir() {
            return None;
        }
        push_probe_stamp_item(out, format!("{label}={}", artifact_stamp(&path)?), cap)?;
        if !is_link && file_type.is_dir() {
            push_probe_stamp_artifacts(&path, &label, out, cap)?;
        }
    }
    Some(())
}

fn artifact_stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.len(), file_modified_nanos(&meta)))
}

fn path_modified_nanos(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) => file_modified_nanos(&meta),
        Err(_) => "-".to_string(),
    }
}

fn file_modified_nanos(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos().to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn read_native_probe_stamp_at(base: &Path) -> Option<(String, String, String)> {
    let raw = std::fs::read_to_string(native_probe_stamp_path_at(base)).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    Some((
        value.get("key")?.as_str()?.to_string(),
        value.get("platform")?.as_str()?.to_string(),
        value.get("arch")?.as_str()?.to_string(),
    ))
}

fn write_native_probe_stamp_at(base: &Path, key: &str, target: &NodeTarget) {
    let path = native_probe_stamp_path_at(base);
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::debug!(
                "CORE_NATIVE_PROBE_STAMP_WRITE_FAILED: {}: {error}",
                parent.display()
            );
            return;
        }
    }
    let value = serde_json::json!({
        "key": key,
        "platform": &target.platform,
        "arch": &target.arch,
    });
    if let Err(error) = std::fs::write(&path, value.to_string()) {
        log::debug!(
            "CORE_NATIVE_PROBE_STAMP_WRITE_FAILED: {}: {error}",
            path.display()
        );
    }
}

async fn detect_node_target(node: &Path, core_root: &Path) -> Result<NodeTarget, String> {
    let node = node.to_path_buf();
    let core_root = core_root.to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        run_command(
            &node,
            &[
                "--input-type=module".into(),
                "-e".into(),
                NODE_PROBE_SCRIPT.into(),
            ],
            &core_root,
        )
    })
    .await
    .map_err(|e| format!("CORE_NODE_PLATFORM_PROBE_FAILED: {e}"))??;
    if !output.status.success() {
        return Err(format!(
            "CORE_NODE_PLATFORM_PROBE_FAILED: {}",
            command_output_tail(&output)
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let Some((platform, arch)) = value.split_once(':') else {
        return Err(format!("CORE_NODE_PLATFORM_UNSUPPORTED: {value}"));
    };
    let supported_platform = matches!(platform, "darwin" | "linux" | "win32");
    let supported_arch = matches!(
        arch,
        "x64" | "arm64" | "ia32" | "arm" | "ppc64" | "riscv64" | "s390x" | "loong64"
    );
    if !supported_platform || !supported_arch {
        return Err(format!("CORE_NODE_PLATFORM_UNSUPPORTED: {value}"));
    }
    Ok(NodeTarget {
        platform: platform.to_string(),
        arch: arch.to_string(),
    })
}

fn run_native_probe(node: &Path, core_root: &Path) -> Result<std::process::Output, String> {
    let script = std::env::temp_dir().join(format!("dsh-native-probe-{}.mjs", std::process::id()));
    std::fs::write(&script, NATIVE_PROBE_SCRIPT).map_err(|error| {
        format!(
            "CORE_NATIVE_PROBE_SCRIPT_FAILED: write {} failed: {error}",
            script.display()
        )
    })?;
    let program = node.as_os_str().to_os_string();
    let args = vec![script.clone().into_os_string()];
    let output = run_process_with_timeout(
        &program,
        &args,
        core_root,
        NATIVE_PROBE_TIMEOUT,
        "native probe",
    );
    let _ = std::fs::remove_file(&script);
    output
}

async fn probe_native_modules(node: &Path, core_root: &Path) -> NativeProbe {
    let node = node.to_path_buf();
    let core_root = core_root.to_path_buf();
    let result = tokio::task::spawn_blocking(move || run_native_probe(&node, &core_root)).await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            match parse_native_probe_failures(&stdout) {
                Some(failures) if failures.is_empty() => NativeProbe::Ready,
                Some(failures) => NativeProbe::Failed(failures),
                None => NativeProbe::Unknown(format!(
                    "probe output is not parseable: {}",
                    command_output_tail(&output)
                )),
            }
        }
        Ok(Ok(output)) => NativeProbe::Unknown(format!(
            "probe exited with {}: {}",
            output.status,
            command_output_tail(&output)
        )),
        Ok(Err(error)) => NativeProbe::Unknown(error),
        Err(error) => NativeProbe::Unknown(error.to_string()),
    }
}

async fn reprobe_after_repair(node: &Path, core_root: &Path) -> Option<NativeProbe> {
    match probe_native_modules(node, core_root).await {
        NativeProbe::Ready => None,
        NativeProbe::Unknown(reason) => {
            log::warn!("CORE_NATIVE_PROBE_UNKNOWN: {reason}");
            None
        }
        probe @ NativeProbe::Failed(_) => Some(probe),
    }
}

async fn native_imports_ready(node: &Path, core_root: &Path) -> bool {
    let node = node.to_path_buf();
    let core_root = core_root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        run_command(
            &node,
            &[
                "--input-type=module".into(),
                "-e".into(),
                NATIVE_IMPORT_SCRIPT.into(),
            ],
            &core_root,
        )
        .is_ok_and(|output| output.status.success())
    })
    .await
    .unwrap_or(false)
}

fn parse_native_probe_failures(stdout: &str) -> Option<Vec<NativeProbeFailure>> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with(NATIVE_PROBE_MARKER))?;
    let json = line.trim_start_matches(NATIVE_PROBE_MARKER).trim();
    serde_json::from_str::<Vec<NativeProbeFailure>>(json).ok()
}



fn native_failure_diagnostic(
    failures: &[NativeProbeFailure],
    node: &Path,
    core_root: &Path,
) -> String {
    if failures.is_empty() {
        return format!(
            "CORE_NATIVE_DEPENDENCY_REPAIR_FAILED: native modules could not be verified in {}",
            core_root.display()
        );
    }
    let abi_failures: Vec<&NativeProbeFailure> =
        failures.iter().filter(|failure| failure.abi).collect();
    if !abi_failures.is_empty() {
        let names = abi_failures
            .iter()
            .map(|failure| failure.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let version = crate::config::get_node_version_of_path(node)
            .unwrap_or_else(|| node.display().to_string());
        return format!(
            "CORE_NATIVE_ABI_MISMATCH: {names} cannot load under Node.js {version}: {detail} The Harness core's native modules were built for a different Node.js ABI; install the Node.js version the core was built with, or reinstall/upgrade the Harness core so it matches the bundled Node.js runtime. (core: {core})",
            names = names,
            version = version,
            detail = collapse_whitespace(&abi_failures[0].message),
            core = core_root.display()
        );
    }
    format!(
        "CORE_NATIVE_DEPENDENCY_REPAIR_FAILED: native modules still cannot load in {}: {}",
        core_root.display(),
        failures
            .iter()
            .map(|failure| format!(
                "{}: {}",
                failure.name,
                collapse_whitespace(&failure.message)
            ))
            .collect::<Vec<_>>()
            .join("; ")
    )
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn rebuild_native_packages(
    core_root: &Path,
    node: &Path,
    packages: &[String],
) -> Result<(), String> {
    let core_root = core_root.to_path_buf();
    let node = node.to_path_buf();
    let packages = packages.to_vec();
    let result = tokio::task::spawn_blocking(move || {
        let (program, mut args) = npm_command(&node);
        args.push(OsString::from("rebuild"));
        args.extend(packages.into_iter().map(OsString::from));
        run_process_with_timeout(
            &program,
            &args,
            &core_root,
            NATIVE_REBUILD_TIMEOUT,
            "npm rebuild",
        )
    })
    .await
    .map_err(|e| format!("CORE_NATIVE_REBUILD_FAILED: {e}"))??;
    if !result.status.success() {
        return Err(format!(
            "CORE_NATIVE_REBUILD_FAILED: npm rebuild exited with {}: {}",
            result.status,
            command_output_tail(&result)
        ));
    }
    Ok(())
}

fn npm_command(node: &Path) -> (OsString, Vec<OsString>) {
    if let Some(cli) = npm_cli_path_for(node) {
        return (node.as_os_str().to_os_string(), vec![cli.into_os_string()]);
    }
    let program = if cfg!(windows) {
        OsString::from("npm.cmd")
    } else {
        OsString::from("npm")
    };
    (program, Vec::new())
}

fn npm_cli_path_for(node: &Path) -> Option<PathBuf> {
    let dir = node.parent()?;
    let candidates = [
        dir.join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js"),
        dir.join("lib")
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js"),
        dir.join("..")
            .join("lib")
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js"),
    ];
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn native_package_plan(core_root: &Path, target: &NodeTarget) -> Vec<String> {
    let node_modules = core_root.join("node_modules");
    let sharp = node_modules.join("sharp");
    let koffi = node_modules.join("koffi");
    let sharp_optional = read_optional_dependencies(&sharp.join("package.json"));
    let koffi_optional = read_optional_dependencies(&koffi.join("package.json"));

    let mut packages = Vec::new();
    let sharp_runtime = format!("@img/sharp-{}-{}", target.platform, target.arch);
    let sharp_libvips = format!("@img/sharp-libvips-{}-{}", target.platform, target.arch);
    let koffi_runtime = format!("@koromix/koffi-{}-{}", target.platform, target.arch);

    for (name, optional) in [
        (sharp_runtime.as_str(), &sharp_optional),
        (sharp_libvips.as_str(), &sharp_optional),
        (koffi_runtime.as_str(), &koffi_optional),
    ] {
        if let Some(version) = optional.get(name) {
            packages.push(format!("{name}@{version}"));
        }
    }
    if !sharp.is_dir() && !packages.iter().any(|p| p == "sharp") {
        packages.push("sharp".to_string());
    }
    if !koffi.is_dir() && !packages.iter().any(|p| p == "koffi") {
        packages.push("koffi".to_string());
    }
    packages
}

fn read_optional_dependencies(path: &Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return HashMap::new();
    };
    value
        .get("optionalDependencies")
        .and_then(|value| value.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|version| (name.clone(), version.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn install_native_packages(
    core_root: &Path,
    target: &NodeTarget,
    packages: &[String],
) -> Result<(), String> {
    let core_root = core_root.to_path_buf();
    let target = target.clone();
    let packages = packages.to_vec();
    let result = tokio::task::spawn_blocking(move || {
        let program = if cfg!(windows) {
            OsString::from("npm.cmd")
        } else {
            OsString::from("npm")
        };
        let mut args = vec![
            OsString::from("install"),
            OsString::from("--no-save"),
            OsString::from("--package-lock=false"),
            OsString::from("--include=optional"),
            OsString::from(format!("--os={}", target.platform)),
            OsString::from(format!("--cpu={}", target.arch)),
        ];
        args.extend(packages.into_iter().map(OsString::from));
        run_process_with_timeout(
            &program,
            &args,
            &core_root,
            NATIVE_REPAIR_TIMEOUT,
            "npm install",
        )
    })
    .await
    .map_err(|e| format!("CORE_NATIVE_DEPENDENCY_REPAIR_FAILED: {e}"))??;
    if !result.status.success() {
        return Err(format!(
            "CORE_NATIVE_DEPENDENCY_REPAIR_FAILED: npm exited with {}: {}",
            result.status,
            command_output_tail(&result)
        ));
    }
    Ok(())
}

fn run_command(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
) -> Result<std::process::Output, String> {
    let mut command = Command::new(program);
    crate::config::apply_node_env(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .output()
        .map_err(|e| format!("CORE_RUNTIME_COMMAND_FAILED: {}: {e}", program.display()))
}

fn run_process_with_timeout(
    program: &OsString,
    args: &[OsString],
    cwd: &Path,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, String> {
    let mut command = Command::new(program);
    crate::config::apply_node_env(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("CORE_NATIVE_DEPENDENCY_REPAIR_SPAWN_FAILED: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("CORE_NATIVE_DEPENDENCY_REPAIR_WAIT_FAILED: {e}"))?
        {
            let stdout = stdout_thread.join().unwrap_or_default();
            let stderr = stderr_thread.join().unwrap_or_default();
            return Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let status = child
                .wait()
                .map_err(|e| format!("CORE_NATIVE_DEPENDENCY_REPAIR_KILL_FAILED: {e}"))?;
            let _ = stdout_thread.join().unwrap_or_default();
            let _ = stderr_thread.join().unwrap_or_default();
            return Err(format!(
                "CORE_NATIVE_DEPENDENCY_REPAIR_TIMEOUT: {label} exceeded {} seconds (exit {status})",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn command_output_tail(output: &std::process::Output) -> String {
    let mut value = String::from_utf8_lossy(&output.stderr).to_string();
    if value.trim().is_empty() {
        value = String::from_utf8_lossy(&output.stdout).to_string();
    }
    value
        .trim()
        .chars()
        .rev()
        .take(2000)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_preparation_requires_shared_links_pruning_and_same_node_stamp() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("dependencies");
        let core = base.join("dsh");
        let node = base.join("node");
        let profiles = root.path().join("profiles");
        let profile = profiles.join("tauri");
        let resources = root.path().join("resources");
        let source = resources.join("node_modules/dsh-tauri");
        for dir in [&core, &profile, &source] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&node, "not an executable").unwrap();
        std::fs::write(profile.join("package.json"), "{}").unwrap();
        std::fs::write(source.join("package.json"), r#"{"name":"dsh-tauri"}"#).unwrap();
        let required = [RuntimeRequiredPlugin {
            name: "dsh-tauri",
            source: Some(PathBuf::from("node_modules/dsh-tauri")),
            version: None,
        }];
        let mut paths = RuntimePreparationPaths {
            source: CoreSource::App,
            base: &base,
            core_root: &core,
            node: &node,
            profile: &profile,
            profiles: &profiles,
            resources: Some(&resources),
            core_version: Some("2.0.0"),
            required_plugins: Some(&required),
        };
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        let destination = core.join("node_modules/dsh-tauri");
        assert_eq!(
            destination.canonicalize().unwrap(),
            source.canonicalize().unwrap()
        );
        let stamp = |paths: &RuntimePreparationPaths<'_>| {
            let key =
                native_probe_stamp_key_at(paths.node, paths.core_root, paths.core_version).unwrap();
            write_native_probe_stamp_at(
                paths.base,
                &key,
                &NodeTarget {
                    platform: "win32".into(),
                    arch: "x64".into(),
                },
            );
        };
        stamp(&paths);
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Ready
        );
        std::fs::write(&node, "a different Node runtime").unwrap();
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        stamp(&paths);

        let name = "@deepseek-ai/dsh-api";
        let anchor = core.join("node_modules").join(name);
        let stale = profile.join("node_modules").join(name);
        std::fs::create_dir_all(&anchor).unwrap();
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(anchor.join("package.json"), r#"{"version":"2.0.0"}"#).unwrap();
        std::fs::write(stale.join("package.json"), r#"{"version":"1.0.0"}"#).unwrap();
        stamp(&paths);
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Ready
        );
        assert!(!stale.exists());
        paths.required_plugins = None;
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        paths.required_plugins = Some(&required);
        std::fs::write(profile.join("package.json"), "invalid").unwrap();
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        std::fs::write(profile.join("package.json"), "{}").unwrap();
        std::fs::write(native_probe_stamp_path_at(&base), r#"{"key":"incomplete"}"#).unwrap();
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        stamp(&paths);
        paths.source = CoreSource::Local;
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        paths.source = CoreSource::App;
        let invalid = [RuntimeRequiredPlugin {
            name: "../escape",
            source: Some(source.clone()),
            version: None,
        }];
        paths.required_plugins = Some(&invalid);
        assert!(prepare_active_runtime_at(&paths)
            .unwrap_err()
            .starts_with("CORE_PLUGIN_NAME_INVALID:"));
        remove_link_only(&destination).unwrap();
        assert!(source.is_dir());

        paths.required_plugins = Some(&required);
        std::fs::write(&destination, "core-owned file").unwrap();
        stamp(&paths);
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "core-owned file"
        );
        std::fs::remove_file(&destination).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let package = destination.join("package.json");
        let marker = destination.join("core-owned.txt");
        std::fs::write(&marker, "keep").unwrap();
        for manifest in [
            None,
            Some("invalid"),
            Some("{}"),
            Some(r#"{"name":"other"}"#),
        ] {
            if let Some(raw) = manifest {
                std::fs::write(&package, raw).unwrap();
            }
            stamp(&paths);
            assert_eq!(
                prepare_active_runtime_at(&paths).unwrap(),
                RuntimePreparation::Defer
            );
            assert_eq!(std::fs::read_to_string(&package).ok().as_deref(), manifest);
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), "keep");
        }
        let manifest = r#"{"name":"dsh-tauri","version":"9.0.0"}"#;
        std::fs::write(&package, manifest).unwrap();
        std::fs::remove_file(native_probe_stamp_path_at(&base)).unwrap();
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Defer
        );
        stamp(&paths);
        assert_eq!(
            prepare_active_runtime_at(&paths).unwrap(),
            RuntimePreparation::Ready
        );
        assert_ne!(
            destination.canonicalize().unwrap(),
            source.canonicalize().unwrap()
        );
        assert_eq!(std::fs::read_to_string(&package).unwrap(), manifest);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "keep");
    }

    fn run_probe_script(node: &std::path::Path, cwd: &std::path::Path) -> std::process::Output {
        let script = cwd.join("dsh-native-probe-test.mjs");
        std::fs::write(&script, NATIVE_PROBE_SCRIPT).unwrap();
        let mut command = Command::new(node);
        command
            .arg(&script)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.output().expect("node must be runnable")
    }

    #[test]
    fn probe_stamp_digest_is_stable_and_size_independent() {
        let modules = std::env::temp_dir().join(format!("dsh-probe-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&modules);
        std::fs::create_dir_all(modules.join(".bin")).unwrap();
        std::fs::create_dir_all(modules.join("@deepseek-ai/dsh-plugin")).unwrap();
        std::fs::create_dir_all(modules.join("sharp/build/Release")).unwrap();
        std::fs::write(modules.join(".bin/dsh"), "shim").unwrap();

        let entries = probe_stamp_entries(&modules);
        assert!(entries.iter().any(|entry| entry.starts_with("sharp=")));
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("sharp/build/Release=")));
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("@deepseek-ai/dsh-plugin=")));
        assert!(!entries.iter().any(|entry| entry.starts_with(".bin")));

        let digest = probe_stamp_modules_digest(&modules).expect("complete fingerprint");
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), "sha256:".len() + 64);
        assert_eq!(digest, probe_stamp_modules_digest(&modules).unwrap());

        std::fs::write(modules.join(".bin/dsh"), "another shim").unwrap();
        assert_eq!(digest, probe_stamp_modules_digest(&modules).unwrap());

        std::fs::create_dir_all(modules.join("plain")).unwrap();
        let with_plain = probe_stamp_modules_digest(&modules).unwrap();
        assert_ne!(digest, with_plain);

        std::fs::write(modules.join("sharp/build/Release/sharp.node"), "native").unwrap();
        assert_ne!(with_plain, probe_stamp_modules_digest(&modules).unwrap());

        std::fs::remove_dir_all(&modules).unwrap();
        assert!(probe_stamp_modules_digest(&modules).is_none());
    }

    #[test]
    fn probe_stamp_detects_in_place_addon_overwrite_beyond_legacy_cap() {
        let modules =
            std::env::temp_dir().join(format!("dsh-probe-stamp-overwrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&modules);
        let release = modules.join("sharp/build/Release");
        std::fs::create_dir_all(&release).unwrap();
        let addon = release.join("sharp.node");
        std::fs::write(&addon, "native-a").unwrap();
        for index in 0..520 {
            std::fs::create_dir_all(modules.join(format!("plain-{index}"))).unwrap();
        }

        let entries = probe_stamp_entries(&modules);
        assert!(
            entries.len() > 512,
            "fixture must exceed the legacy entry cap"
        );
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("sharp/build/Release/sharp.node=8:")));

        let digest = probe_stamp_modules_digest(&modules).expect("complete fingerprint");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&addon, "native-b").unwrap();
        assert_ne!(digest, probe_stamp_modules_digest(&modules).unwrap());

        std::fs::remove_dir_all(&modules).unwrap();
    }

    #[test]
    fn probe_stamp_requires_readable_artifact_metadata() {
        let dir =
            std::env::temp_dir().join(format!("dsh-probe-stamp-artifact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let addon = dir.join("sharp.node");

        assert!(artifact_stamp(&addon).is_none());
        std::fs::write(&addon, "native").unwrap();
        assert!(artifact_stamp(&addon).is_some());
        assert!(push_probe_stamp_artifacts(
            &dir,
            "sharp/build/Release",
            &mut Vec::new(),
            NATIVE_PROBE_STAMP_MAX_ENTRIES
        )
        .is_some());

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(artifact_stamp(&addon).is_none());
        assert!(push_probe_stamp_artifacts(
            &dir,
            "sharp/build/Release",
            &mut Vec::new(),
            NATIVE_PROBE_STAMP_MAX_ENTRIES
        )
        .is_none());
    }

    #[test]
    fn probe_stamp_detects_in_place_overwrite_below_nested_artifact_directory() {
        let modules =
            std::env::temp_dir().join(format!("dsh-probe-stamp-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&modules);
        let nested = modules.join("sharp/prebuilds/win32-x64");
        std::fs::create_dir_all(&nested).unwrap();
        let addon = nested.join("addon.node");
        std::fs::write(&addon, "native-a").unwrap();

        let entries = probe_stamp_entries(&modules);
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("sharp/prebuilds=")));
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("sharp/prebuilds/win32-x64=")));
        assert!(entries
            .iter()
            .any(|entry| entry.starts_with("sharp/prebuilds/win32-x64/addon.node=8:")));

        let digest = probe_stamp_modules_digest(&modules).expect("complete fingerprint");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&addon, "native-b").unwrap();
        assert_ne!(digest, probe_stamp_modules_digest(&modules).unwrap());

        std::fs::remove_dir_all(&modules).unwrap();
    }

    #[test]
    fn probe_stamp_cap_aborts_traversal_immediately() {
        let dir = std::env::temp_dir().join(format!("dsh-probe-stamp-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..100 {
            std::fs::create_dir_all(dir.join(format!("pkg-{index}"))).unwrap();
        }

        let mut capped = Vec::new();
        assert!(collect_probe_stamp_entries(&dir, &mut capped, 4).is_none());
        assert_eq!(
            capped.len(),
            5,
            "traversal must stop right after crossing the cap"
        );

        let mut full = Vec::new();
        assert!(
            collect_probe_stamp_entries(&dir, &mut full, NATIVE_PROBE_STAMP_MAX_ENTRIES).is_some()
        );
        assert_eq!(full.len(), 100);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn probe_stamp_rejects_a_directory_symlink_it_cannot_descend() {
        let root =
            std::env::temp_dir().join(format!("dsh-probe-stamp-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("pkg/prebuilds/real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("addon.node"), "native").unwrap();
        let linked = root.join("pkg/prebuilds/win32-x64");
        let canonical_real = real.canonicalize().unwrap();
        create_directory_link(&canonical_real, &linked)
            .unwrap_or_else(|e| panic!("link must be created without privileges: {e}"));
        assert!(
            std::fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link must be treated as a link"
        );
        let mut out = Vec::new();
        let rejected = push_probe_stamp_artifacts(
            &root.join("pkg/prebuilds"),
            "pkg/prebuilds",
            &mut out,
            NATIVE_PROBE_STAMP_MAX_ENTRIES,
        )
        .is_none();
        let _ = std::fs::remove_dir(&linked);
        let _ = std::fs::remove_dir_all(&root);
        assert!(rejected);
    }

    fn probe_stamp_entries(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        collect_probe_stamp_entries(dir, &mut out, NATIVE_PROBE_STAMP_MAX_ENTRIES).unwrap();
        out.sort_unstable();
        out
    }

    #[test]
    fn probe_stamp_digest_length_does_not_grow_with_entries() {
        let wide = (0..4000)
            .map(|index| format!("pkg-{index}=1"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(probe_stamp_digest(&wide).len(), "sha256:".len() + 64);
        assert_ne!(probe_stamp_digest(&wide), probe_stamp_digest("pkg-0=1"));
    }

    #[test]
    fn profile_plugin_links_do_not_survive_in_shared_core() {
        fn package(modules: &Path, name: &str, version: &str) {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("package.json"),
                serde_json::json!({"name": name, "version": version}).to_string(),
            )
            .unwrap();
        }
        let root = tempfile::tempdir().unwrap();
        let profiles = root.path().join("profiles");
        let modules = root.path().join("core/node_modules");
        for (id, version) in [("a", "1.0.0"), ("b", "2.0.0")] {
            let profile = profiles.join(id);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(profile.join("package.json"), r#"{"dependencies":{"same-plugin":"*","@scope/plugin":"*","dsh-tauri":"*","core-owned":"*"}}"#).unwrap();
            for name in ["same-plugin", "@scope/plugin", "dsh-tauri", "core-owned"] {
                package(&profile.join("node_modules"), name, version);
            }
        }
        std::fs::create_dir_all(modules.join("@scope")).unwrap();
        for (name, id) in [
            ("same-plugin", "a"),
            ("@scope/plugin", "b"),
            ("dsh-tauri", "a"),
        ] {
            create_directory_link(
                &profiles
                    .join(id)
                    .join("node_modules")
                    .join(name)
                    .canonicalize()
                    .unwrap(),
                &modules.join(name),
            )
            .unwrap();
        }
        package(&modules, "core-owned", "9.0.0");
        let internal = HashSet::from(["dsh-tauri".to_string()]);
        remove_profile_plugin_links(&modules, &profiles, &internal).unwrap();
        remove_profile_plugin_links(&modules, &profiles, &internal).unwrap();
        assert!(std::fs::symlink_metadata(modules.join("same-plugin")).is_err());
        assert!(std::fs::symlink_metadata(modules.join("@scope/plugin")).is_err());
        assert_eq!(
            read_package_version(&modules.join("dsh-tauri/package.json")).as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            read_package_version(&modules.join("core-owned/package.json")).as_deref(),
            Some("9.0.0")
        );
        assert_eq!(
            read_package_version(&profiles.join("a/node_modules/same-plugin/package.json"))
                .as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            read_package_version(&profiles.join("b/node_modules/same-plugin/package.json"))
                .as_deref(),
            Some("2.0.0")
        );
        remove_link_only(&modules.join("dsh-tauri")).unwrap();
    }

    #[test]
    fn package_names_allow_plain_and_scoped_names_only() {
        assert!(is_safe_package_name("dshmarket"));
        assert!(is_safe_package_name("@scope/plugin-name"));
        assert!(!is_safe_package_name("../escape"));
        assert!(!is_safe_package_name("@scope/../escape"));
        assert!(!is_safe_package_name("C:\\escape"));
        assert!(!is_safe_package_name("/absolute"));
        assert!(!is_safe_package_name("@/missing"));
    }

    #[test]
    fn native_plan_uses_manifest_versions_not_constants() {
        let root = std::env::temp_dir().join(format!("dsh-native-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("node_modules/sharp")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/koffi")).unwrap();
        std::fs::write(
            root.join("node_modules/sharp/package.json"),
            r#"{"optionalDependencies":{"@img/sharp-darwin-arm64":"0.9.0","@img/sharp-libvips-darwin-arm64":"1.2.0"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("node_modules/koffi/package.json"),
            r#"{"optionalDependencies":{"@koromix/koffi-darwin-arm64":"8.7.0"}}"#,
        )
        .unwrap();
        let plan = native_package_plan(
            &root,
            &NodeTarget {
                platform: "darwin".into(),
                arch: "arm64".into(),
            },
        );
        assert_eq!(
            plan,
            vec![
                "@img/sharp-darwin-arm64@0.9.0",
                "@img/sharp-libvips-darwin-arm64@1.2.0",
                "@koromix/koffi-darwin-arm64@8.7.0",
            ]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn create_directory_link_falls_back_to_junction_without_privileges() {
        let root = std::env::temp_dir().join(format!("dsh-runtime-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("source-pkg");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("package.json"), r#"{"name":"dsh-tauri"}"#).unwrap();
        let destination = root.join("node_modules").join("dsh-tauri");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();

        let canonical_source = source.canonicalize().unwrap();
        create_directory_link(&canonical_source, &destination)
            .unwrap_or_else(|e| panic!("link must be created without privileges: {e}"));

        let metadata = std::fs::symlink_metadata(&destination).unwrap();
        assert!(
            metadata.file_type().is_symlink(),
            "junction must be treated as a link"
        );
        let resolved = std::fs::read_link(&destination).unwrap();
        assert_eq!(resolved.canonicalize().unwrap(), canonical_source);

        let matches = std::fs::read_link(&destination)
            .map(|target| {
                let resolved = if target.is_absolute() {
                    target
                } else {
                    destination.parent().unwrap_or(Path::new(".")).join(target)
                };
                resolved.canonicalize().ok().as_deref() == Some(canonical_source.as_path())
            })
            .unwrap_or(false);
        assert!(matches, "existing junction must match its source");

        remove_link_only(&destination).unwrap();
        assert!(!destination.exists());
        assert!(canonical_source.is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn create_directory_junction_accepts_mixed_separators_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("dsh-junction-direct-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src-tauri")).unwrap();
        let source = root.join("src-tauri/../@scope/package");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("package.json"), r#"{"name":"dsh-tauri"}"#).unwrap();
        let destination = root.join("node_modules").join("dsh-tauri");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();

        let canonical_source = source.canonicalize().unwrap();
        create_directory_junction(&source, &destination)
            .unwrap_or_else(|e| panic!("junction must be creatable without privileges: {e}"));
        let manifest = std::fs::read_to_string(destination.join("package.json"));
        remove_link_only(&destination).unwrap();
        assert_eq!(manifest.unwrap(), r#"{"name":"dsh-tauri"}"#);
        create_directory_junction(&source, &destination).unwrap();

        let metadata = std::fs::symlink_metadata(&destination).unwrap();
        assert!(
            metadata.file_type().is_symlink(),
            "junction must be treated as a link"
        );
        assert_eq!(
            std::fs::canonicalize(&destination).unwrap(),
            canonical_source,
            "junction must resolve to the source directory"
        );

        remove_link_only(&destination).unwrap();
        create_directory_junction(&canonical_source, &destination)
            .unwrap_or_else(|e| panic!("junction recreation must succeed: {e}"));
        let matches = std::fs::read_link(&destination)
            .ok()
            .map(|target| {
                let resolved = if target.is_absolute() {
                    target
                } else {
                    destination.parent().unwrap_or(Path::new(".")).join(target)
                };
                resolved.canonicalize().ok().as_deref() == Some(canonical_source.as_path())
            })
            .unwrap_or(false);
        assert!(matches, "existing junction must match its source");

        remove_link_only(&destination).unwrap();
        assert!(!destination.exists());
        assert!(canonical_source.is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_probe_script_matches_marker_and_checks_abi() {
        assert!(NATIVE_PROBE_SCRIPT.contains(NATIVE_PROBE_MARKER));
        assert!(NATIVE_PROBE_SCRIPT.contains("NODE_MODULE_VERSION"));
        assert!(NATIVE_PROBE_SCRIPT.contains("binding.gyp"));
        assert!(NATIVE_PROBE_SCRIPT.contains("'sharp'"));
        assert!(NATIVE_PROBE_SCRIPT.contains("'koffi'"));
    }

    #[test]
    fn native_probe_failures_parse_from_marker_line_ignoring_noise() {
        let stdout = concat!(
            "some addon chatter\n",
            "__DSH_NATIVE_PROBE__[{\"kind\":\"addon\",\"name\":\"fs-ext\",\"message\":\"The module 'x' was compiled against a different Node.js version using NODE_MODULE_VERSION 137. This version of Node.js requires NODE_MODULE_VERSION 141.\",\"abi\":true},",
            "{\"kind\":\"import\",\"name\":\"sharp\",\"message\":\"Cannot find module 'sharp'\",\"abi\":false}]\n"
        );
        let failures = parse_native_probe_failures(stdout).expect("marker line must parse");
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].name, "fs-ext");
        assert!(failures[0].abi);
        assert!(!failures[0].is_platform_import());
        assert!(!failures[1].abi);
        assert!(failures[1].is_platform_import());
    }

    #[test]
    fn native_probe_parse_returns_none_without_marker() {
        assert!(parse_native_probe_failures("no marker here").is_none());
        assert!(parse_native_probe_failures("__DSH_NATIVE_PROBE__not json").is_none());
    }

    #[test]
    fn native_probe_classifies_abi_and_platform_failures() {
        let probe = NativeProbe::Failed(vec![
            NativeProbeFailure {
                kind: "addon".into(),
                name: "fs-ext".into(),
                message: "NODE_MODULE_VERSION 137".into(),
                abi: true,
            },
            NativeProbeFailure {
                kind: "addon".into(),
                name: "fs-ext".into(),
                message: "duplicate entry".into(),
                abi: true,
            },
            NativeProbeFailure {
                kind: "import".into(),
                name: "koffi".into(),
                message: "Cannot find module".into(),
                abi: false,
            },
        ]);
        assert!(!probe.is_ready());
        assert!(probe.has_abi_mismatch());
        assert!(probe.has_platform_import_failure());
        assert_eq!(probe.abi_packages(), vec!["fs-ext".to_string()]);
        assert!(NativeProbe::Ready.abi_packages().is_empty());
        assert!(!NativeProbe::Unknown("boom".into()).has_abi_mismatch());
    }

    /// ABI 诊断必须点名模块、折叠 Node 的跨行报错并给出可操作建议（issue #441）。
    #[test]
    fn native_failure_diagnostic_reports_abi_mismatch() {
        let failures = vec![NativeProbeFailure {
            kind: "addon".into(),
            name: "fs-ext".into(),
            message: "The module 'fs_ext.node'\nwas compiled against a different Node.js version using\nNODE_MODULE_VERSION 137.".into(),
            abi: true,
        }];
        let diagnostic = native_failure_diagnostic(
            &failures,
            Path::new("C:\\missing\\node.exe"),
            Path::new("C:\\core"),
        );
        assert!(diagnostic.starts_with("CORE_NATIVE_ABI_MISMATCH:"));
        assert!(diagnostic.contains("fs-ext"));
        assert!(diagnostic.contains("NODE_MODULE_VERSION 137"));
        assert!(
            !diagnostic.contains('\n'),
            "diagnostic must be a single line"
        );
        assert!(diagnostic.contains("install the Node.js version the core was built with"));
    }

    #[test]
    fn native_failure_diagnostic_lists_non_abi_failures() {
        let failures = vec![NativeProbeFailure {
            kind: "import".into(),
            name: "sharp".into(),
            message: "Cannot find module 'sharp'".into(),
            abi: false,
        }];
        let diagnostic =
            native_failure_diagnostic(&failures, Path::new("node"), Path::new("C:\\core"));
        assert!(diagnostic.starts_with("CORE_NATIVE_DEPENDENCY_REPAIR_FAILED:"));
        assert!(diagnostic.contains("sharp: Cannot find module 'sharp'"));
    }

    #[test]
    fn collapse_whitespace_folds_multiline_messages() {
        assert_eq!(collapse_whitespace("a\n  b\t c"), "a b c");
    }

    #[test]
    fn npm_command_prefers_node_local_npm_cli() {
        let root = std::env::temp_dir().join(format!("dsh-npm-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin_dir = root.join("node-v22.22.0-win-x64");
        std::fs::create_dir_all(bin_dir.join("node_modules").join("npm").join("bin")).unwrap();
        let npm_cli = bin_dir
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js");
        std::fs::write(&npm_cli, "// npm").unwrap();
        let node = bin_dir.join("node.exe");

        let (program, args) = npm_command(&node);
        assert_eq!(program, node.as_os_str().to_os_string());
        assert_eq!(args, vec![npm_cli.clone().into_os_string()]);
        assert_eq!(npm_cli_path_for(&node), Some(npm_cli));

        let bare = root.join("bare").join("node.exe");
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        let (program, args) = npm_command(&bare);
        assert!(program == "npm.cmd" || program == "npm");
        assert!(args.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_probe_script_runs_on_real_node() {
        let Some(node) = crate::config::get_local_node_path() else {
            return;
        };
        let root = std::env::temp_dir().join(format!("dsh-native-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("node_modules")).unwrap();

        let output = run_probe_script(&node, &root);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "probe script must exit 0: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let failures =
            parse_native_probe_failures(&stdout).expect("probe must emit the marker line");
        assert!(failures.iter().all(|failure| !failure.abi));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// （issue #441 的 fs-ext 形态：binding.gyp + build/Release/*.node，包入口抛 ABI 错误）。
    #[test]
    fn native_probe_detects_abi_mismatch_package() {
        let Some(node) = crate::config::get_local_node_path() else {
            return;
        };
        let root = std::env::temp_dir().join(format!("dsh-native-abi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pkg = root.join("node_modules").join("fs-ext");
        std::fs::create_dir_all(pkg.join("build").join("Release")).unwrap();
        std::fs::write(pkg.join("binding.gyp"), "{}").unwrap();
        std::fs::write(
            pkg.join("build").join("Release").join("fs_ext.node"),
            "stub",
        )
        .unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"fs-ext","main":"index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            pkg.join("index.js"),
            "throw new Error(\"was compiled against a different Node.js version using\\nNODE_MODULE_VERSION 137. This version of Node.js requires\\nNODE_MODULE_VERSION 141.\");",
        )
        .unwrap();

        let output = run_probe_script(&node, &root);
        let failures = parse_native_probe_failures(&String::from_utf8_lossy(&output.stdout))
            .expect("marker line");
        let probe = NativeProbe::Failed(failures);
        assert!(
            probe.has_abi_mismatch(),
            "fs-ext must be reported as an ABI mismatch"
        );
        assert_eq!(probe.abi_packages(), vec!["fs-ext".to_string()]);
        let diagnostic = native_failure_diagnostic(probe.failures(), &node, &root);
        assert!(diagnostic.starts_with("CORE_NATIVE_ABI_MISMATCH:"));
        assert!(diagnostic.contains("fs-ext"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_undeclared_core_package_is_pruned() {
        let root = std::env::temp_dir().join(format!("dsh-stale-core-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile_modules = root.join("profiles/tauri/node_modules");
        let anchor_modules = root.join("dependencies/dsh/node_modules");

        write_package_version(&profile_modules, "dsh-settings", "0.1.5-rc.2");
        write_package_version(&anchor_modules, "dsh-settings", "0.1.7-rc.2");

        prune_stale_core_entries(
            &root.join("profiles/tauri"),
            &anchor_modules,
            &HashSet::new(),
        )
        .unwrap();

        assert!(
            !profile_modules
                .join("@deepseek-ai/dsh-settings/package.json")
                .exists(),
            "mismatched undeclared core package must be removed from the profile"
        );
        assert!(
            anchor_modules
                .join("@deepseek-ai/dsh-settings/package.json")
                .is_file(),
            "the anchor copy must never be touched"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn matching_or_declared_core_packages_are_kept() {
        let root = std::env::temp_dir().join(format!("dsh-stale-core-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile_modules = root.join("profiles/tauri/node_modules");
        let anchor_modules = root.join("dependencies/dsh/node_modules");

        write_package_version(&profile_modules, "dsh-settings", "0.1.7-rc.2");
        write_package_version(&anchor_modules, "dsh-settings", "0.1.7-rc.2");
        write_package_version(&profile_modules, "dsh-tools", "0.1.5-rc.2");
        write_package_version(&anchor_modules, "dsh-tools", "0.1.7-rc.2");
        write_package_version(&root.join("other"), "dsh-settings", "0.1.5-rc.2");
        std::fs::create_dir_all(profile_modules.join("@deepseek-ai/dsh-orphan")).unwrap();

        let declared = HashSet::from(["@deepseek-ai/dsh-tools".to_string()]);
        prune_stale_core_entries(&root.join("profiles/tauri"), &anchor_modules, &declared).unwrap();

        assert!(
            profile_modules
                .join("@deepseek-ai/dsh-settings/package.json")
                .is_file(),
            "version-matching core package must be kept"
        );
        assert!(
            profile_modules
                .join("@deepseek-ai/dsh-tools/package.json")
                .is_file(),
            "declared core package must be kept"
        );
        assert!(
            profile_modules.join("@deepseek-ai/dsh-orphan").is_dir(),
            "entry without a readable version must be left alone"
        );
        assert!(
            root.join("other/@deepseek-ai/dsh-settings/package.json")
                .is_file(),
            "only the @deepseek-ai scope is inspected"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shared_framework_libraries_and_third_party_plugins_are_kept() {
        let root =
            std::env::temp_dir().join(format!("dsh-stale-core-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile_modules = root.join("profiles/tauri/node_modules");
        let anchor_modules = root.join("dependencies/dsh/node_modules");

        for name in ["schemastery", "cosmokit", "cordis", "dshmarket"] {
            write_package_version(&profile_modules, name, "0.1.5-rc.2");
            write_package_version(&anchor_modules, name, "0.1.7-rc.2");
        }

        prune_stale_core_entries(
            &root.join("profiles/tauri"),
            &anchor_modules,
            &HashSet::new(),
        )
        .unwrap();

        for name in ["schemastery", "cosmokit", "cordis", "dshmarket"] {
            assert!(
                profile_modules
                    .join(format!("@deepseek-ai/{name}/package.json"))
                    .is_file(),
                "{name} is outside the core family and must be kept"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn stale_core_package_symlink_is_removed_without_touching_source() {
        let root = std::env::temp_dir().join(format!("dsh-stale-core-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile_modules = root.join("profiles/tauri/node_modules");
        let anchor_modules = root.join("dependencies/dsh/node_modules");
        let source = root.join("fallback/@deepseek-ai/dsh-settings");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("package.json"), r#"{"version":"0.1.5-rc.2"}"#).unwrap();
        std::fs::create_dir_all(profile_modules.join("@deepseek-ai")).unwrap();
        std::os::unix::fs::symlink(&source, profile_modules.join("@deepseek-ai/dsh-settings"))
            .unwrap();
        write_package_version(&anchor_modules, "dsh-settings", "0.1.7-rc.2");

        prune_stale_core_entries(
            &root.join("profiles/tauri"),
            &anchor_modules,
            &HashSet::new(),
        )
        .unwrap();

        assert!(
            !profile_modules.join("@deepseek-ai/dsh-settings").exists(),
            "stale link must be removed"
        );
        assert!(
            source.join("package.json").is_file(),
            "the link source must survive"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn redirected_scope_is_skipped_entirely() {
        let root =
            std::env::temp_dir().join(format!("dsh-stale-core-redirect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile_modules = root.join("profiles/tauri/node_modules");
        let anchor_modules = root.join("dependencies/dsh/node_modules");
        let outside = root.join("outside/@deepseek-ai");
        write_package_version(&root.join("outside"), "dsh-settings", "0.1.5-rc.2");
        write_package_version(&anchor_modules, "dsh-settings", "0.1.7-rc.2");
        std::fs::create_dir_all(&profile_modules).unwrap();
        std::os::unix::fs::symlink(&outside, profile_modules.join("@deepseek-ai")).unwrap();

        prune_stale_core_entries(
            &root.join("profiles/tauri"),
            &anchor_modules,
            &HashSet::new(),
        )
        .unwrap();

        assert!(
            outside.join("dsh-settings/package.json").is_file(),
            "a redirected scope must never be pruned through"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn redirected_node_modules_is_skipped_entirely() {
        let root = std::env::temp_dir().join(format!("dsh-stale-core-nm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("profiles/tauri");
        let anchor_modules = root.join("dependencies/dsh/node_modules");
        let outside_modules = root.join("outside/node_modules");
        write_package_version(&outside_modules, "dsh-settings", "0.1.5-rc.2");
        write_package_version(&anchor_modules, "dsh-settings", "0.1.7-rc.2");
        std::fs::create_dir_all(&profile).unwrap();
        std::os::unix::fs::symlink(&outside_modules, profile.join("node_modules")).unwrap();

        prune_stale_core_entries(&profile, &anchor_modules, &HashSet::new()).unwrap();

        assert!(
            outside_modules
                .join("@deepseek-ai/dsh-settings/package.json")
                .is_file(),
            "an external node_modules must never be pruned through"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn write_package_version(modules: &Path, name: &str, version: &str) {
        let dir = modules.join("@deepseek-ai").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            format!(r#"{{"name":"@deepseek-ai/{name}","version":"{version}"}}"#),
        )
        .unwrap();
    }
}
