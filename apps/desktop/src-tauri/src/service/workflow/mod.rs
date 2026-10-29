
pub mod instances;
pub mod status;
pub mod utils;
pub(crate) mod win_inspector;
#[cfg(windows)]
pub(crate) mod win_spawn;

pub(crate) mod early;
mod early_plan;
mod health;
mod heap;
mod install;
mod launch;
mod process;
mod startup;
mod sweep;

pub use health::{proxy_health_check, recheck_ownership, HealthCheck};
pub(crate) use heap::effective_heap_limit_mb;
pub use install::install;
pub use launch::{launch, restart, start};
pub use process::{
    acquire_core_transition, has_owned_process, owned_process_pid, stop, stop_for_installer,
    stop_on_exit, terminate_stale_harness_processes,
};
pub use sweep::sweep_orphan_harness;
