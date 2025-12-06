use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;

pub(crate) fn launch_lock_path() -> PathBuf {
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(runtime).join("hyprsets.run.lock");
    }
    env::temp_dir().join("hyprsets.run.lock")
}

pub(crate) fn acquire_launch_lock(verbose: bool) -> Result<fs::File> {
    let path = launch_lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create launch lock dir: {}", parent.display()))?;
    }

    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&path)
        .with_context(|| format!("failed to open launch lock: {}", path.display()))?;

    if verbose {
        println!(" acquiring launch lock at {}...", path.display());
    }
    file.lock_exclusive()
        .with_context(|| format!("failed to acquire launch lock at {}", path.display()))?;
    if verbose {
        println!(" launch lock acquired");
    }
    Ok(file)
}
