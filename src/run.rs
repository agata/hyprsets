mod actions_layout;
mod actions_workspace;
mod lock;
mod util;

pub use actions_workspace::{run_workset, workspace_cleanup_status};

pub(crate) use lock::acquire_launch_lock;
pub(crate) use util::build_exec_command;

use std::time::Duration;

pub(crate) const SLOT_LAUNCH_DELAY: Duration = Duration::from_secs(1);
pub(crate) const WINDOW_APPEAR_TIMEOUT: Duration = Duration::from_secs(8);
pub(crate) const WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const WORKSPACE_SWITCH_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const HYPR_SPLIT_MIN: f32 = 0.1;
pub(crate) const HYPR_SPLIT_MAX: f32 = 1.9;
