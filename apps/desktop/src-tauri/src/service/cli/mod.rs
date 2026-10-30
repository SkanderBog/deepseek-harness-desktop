mod core;
mod path;
mod shim;

pub use core::{CliLinkStatus, ensure, ensure_shims, get_status, remove};
// 数据目录（issue #871）在 Unix 上复用同一套「标记块 + 备份写回」原语，
#[cfg(windows)]
pub(crate) use path::find_user_pnpm_executable;
#[cfg(windows)]
pub(crate) use path::{
    delete_user_env, expand_env, notify_environment_change, read_user_env, write_user_env,
};
pub use path::{find_user_pnpm, get_bin_dir, pnpm_env_value};
pub(crate) use path::{find_user_pnpm_from, get_bin_dir_from};
#[cfg(unix)]
pub(crate) use path::{strip_rc_block, upsert_rc_block, write_rc_with_backup};
pub use shim::is_generated_shim;
