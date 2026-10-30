use crate::AppHandle;
use crate::config;
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Dsh,
    Pnpm,
}

impl InstallKind {
    pub fn dependency_key(self) -> &'static str {
        match self {
            InstallKind::Dsh => config::dependencies::DEP_DSH,
            InstallKind::Pnpm => config::dependencies::DEP_PNPM,
        }
    }
}

#[async_trait]
pub trait Installable: Send + Sync {
    fn kind(&self) -> InstallKind;
    fn title(&self) -> &str;
    fn check_installed(&self, app: &AppHandle) -> bool;
    fn get_download_url(&self) -> Result<String, String>;
    fn get_install_path(&self, app: &AppHandle) -> PathBuf;
    fn record_mapping(&self, app: &AppHandle);
}

pub fn tasks() -> Vec<Box<dyn Installable>> {
    vec![Box::new(Dsh), Box::new(Pnpm)]
}

pub fn record_mappings(app: &AppHandle) {
    for task in tasks() {
        task.record_mapping(app);
    }
}

pub struct Dsh;

#[async_trait]
impl Installable for Dsh {
    fn kind(&self) -> InstallKind {
        InstallKind::Dsh
    }
    fn title(&self) -> &str {
        "Harness 核心"
    }
    fn get_download_url(&self) -> Result<String, String> {
        config::get_dsh_download_url()
    }
    fn get_install_path(&self, app: &AppHandle) -> PathBuf {
        config::get_dsh_install_path(app)
    }
    fn check_installed(&self, app: &AppHandle) -> bool {
        config::dependencies::binary_present(app, config::dependencies::DEP_DSH)
    }
    fn record_mapping(&self, app: &AppHandle) {
        let active = config::dependencies::active_root(app, config::dependencies::DEP_DSH);
        if config::dependencies::binary_present(app, config::dependencies::DEP_DSH) {
            config::dependencies::record(app, InstallKind::Dsh.dependency_key(), Some(active));
        } else if crate::service::core::active_version(app).is_some() {
            config::dependencies::record(app, InstallKind::Dsh.dependency_key(), None);
        }
    }
}

pub struct Pnpm;

#[async_trait]
impl Installable for Pnpm {
    fn kind(&self) -> InstallKind {
        InstallKind::Pnpm
    }
    fn title(&self) -> &str {
        "pnpm 包管理器"
    }
    fn get_download_url(&self) -> Result<String, String> {
        Ok(config::get_pnpm_download_url())
    }
    fn get_install_path(&self, app: &AppHandle) -> PathBuf {
        config::get_pnpm_install_path(app)
    }
    fn check_installed(&self, app: &AppHandle) -> bool {
        if crate::service::cli::find_user_pnpm(app).is_some() {
            log::info!("Detected user-installed pnpm, skipping bundled pnpm");
            return true;
        }
        config::get_pnpm_binary_path(app).exists()
    }
    fn record_mapping(&self, app: &AppHandle) {
        let managed = config::get_pnpm_install_path(app);
        let bundled_present = config::get_pnpm_binary_path(app).exists();
        if bundled_present {
            config::dependencies::record(app, InstallKind::Pnpm.dependency_key(), Some(managed));
        } else if crate::service::cli::find_user_pnpm(app).is_some() {
            config::dependencies::record(app, InstallKind::Pnpm.dependency_key(), None);
        }
    }
}
