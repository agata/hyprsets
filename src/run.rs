use anyhow::{Context, Result, bail};
use hyprland::{
    data::{Clients, Monitor, WorkspaceBasic},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{Address, HyprData, HyprDataActive},
};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::config::{LayoutNode, SplitDirection, Workset};

const SLOT_LAUNCH_DELAY: Duration = Duration::from_secs(1);
const WINDOW_APPEAR_TIMEOUT: Duration = Duration::from_secs(8);
const WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(100);
const HYPR_SPLIT_MIN: f32 = 0.1;
const HYPR_SPLIT_MAX: f32 = 1.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceCleanAction {
    Proceed,
    Cancelled,
}

#[derive(Clone, Debug)]
struct CloseCandidate {
    address: Address,
    class: String,
    title: String,
}

#[derive(Clone, Debug)]
struct WorkspaceContext {
    workspace: WorkspaceBasic,
    is_special: bool,
}

#[derive(Clone, Debug)]
struct ActiveWorkspaceState {
    context: WorkspaceContext,
    candidates: Vec<CloseCandidate>,
    initial_clients: usize,
}

#[derive(Clone, Debug)]
pub struct WorkspaceCleanupStatus {
    pub workspace_name: String,
    pub closable_windows: usize,
}

impl WorkspaceContext {
    fn from_basic(workspace: WorkspaceBasic) -> Self {
        let is_special = workspace.id <= 0 || workspace.name.starts_with("special");
        Self {
            workspace,
            is_special,
        }
    }

    fn matches(&self, other: &WorkspaceBasic) -> bool {
        other.id == self.workspace.id
            || (!self.workspace.name.is_empty() && other.name == self.workspace.name)
    }

    fn label(&self) -> String {
        let prefix = if self.is_special {
            "special workspace"
        } else {
            "workspace"
        };
        if self.workspace.name.is_empty() {
            format!("{prefix} {}", self.workspace.id)
        } else {
            format!(
                "{prefix} {} (id {})",
                self.workspace.name, self.workspace.id
            )
        }
    }
}

fn build_exec_command<'a>(
    base_cmd: &str,
    cwd: Option<&Path>,
    env_layers: impl IntoIterator<Item = &'a HashMap<String, String>>,
) -> String {
    let mut exec = String::new();

    if let Some(dir) = cwd {
        exec.push_str("cd ");
        exec.push_str(&shell_escape(&dir.to_string_lossy()));
        exec.push_str(" && ");
    }

    let mut has_env = false;
    for env in env_layers {
        for (key, value) in env {
            if has_env {
                exec.push(' ');
            }
            exec.push_str(key);
            exec.push('=');
            exec.push_str(&shell_escape(value));
            has_env = true;
        }
    }

    if has_env {
        exec.push(' ');
    }

    exec.push_str(base_cmd);
    exec
}

fn shell_escape(raw: &str) -> String {
    let mut escaped = String::from("'");
    for ch in raw.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

fn resolve_active_workspace(verbose: bool) -> Result<(WorkspaceContext, Clients)> {
    let monitor = Monitor::get_active().context("failed to get active monitor from Hyprland")?;
    let clients = Clients::get().context("failed to list Hyprland clients")?;

    if let Some(focused) = clients.iter().find(|c| c.focus_history_id == 0) {
        let ctx = WorkspaceContext::from_basic(focused.workspace.clone());
        if verbose {
            println!(
                " active workspace determined from focused window: {}",
                ctx.label()
            );
        }
        return Ok((ctx, clients));
    }

    let special_ctx = WorkspaceContext::from_basic(monitor.special_workspace.clone());
    let special_active =
        special_ctx.workspace.id != 0 || !special_ctx.workspace.name.trim().is_empty();
    if special_active {
        if verbose {
            println!(
                " active special workspace on focused monitor: {}",
                special_ctx.label()
            );
        }
        return Ok((special_ctx, clients));
    }

    let ctx = WorkspaceContext::from_basic(monitor.active_workspace.clone());
    if verbose {
        println!(" active workspace: {}", ctx.label());
    }
    Ok((ctx, clients))
}

pub fn run_workset(ws: &Workset, verbose: bool, preconfirm_clean: bool) -> Result<()> {
    match clean_active_workspace(verbose, preconfirm_clean)
        .context("failed to clean active workspace before launch")?
    {
        WorkspaceCleanAction::Proceed => {}
        WorkspaceCleanAction::Cancelled => {
            println!("Launch cancelled: existing windows were kept open.");
            return Ok(());
        }
    }

    if let Some(layout) = &ws.layout {
        println!("launching workset '{}' with layout...", ws.name);
        run_layout(layout, ws, verbose)
            .with_context(|| format!("failed to launch layout (id: {})", ws.id))?;
    } else {
        println!("launching workset '{}' (commands sequential)...", ws.name);
        run_commands(ws, verbose)
            .with_context(|| format!("failed to run commands (id: {})", ws.id))?;
    }
    Ok(())
}

pub fn workspace_cleanup_status() -> Result<WorkspaceCleanupStatus> {
    let state = collect_active_workspace_state(false)?;
    Ok(WorkspaceCleanupStatus {
        workspace_name: state.context.workspace.name.clone(),
        closable_windows: state.candidates.len(),
    })
}

fn clean_active_workspace(verbose: bool, preconfirm: bool) -> Result<WorkspaceCleanAction> {
    let state = collect_active_workspace_state(verbose)?;
    let label = state.context.label();

    if state.candidates.is_empty() {
        if verbose {
            println!("closing skipped: {} had no windows", label);
        }
        return Ok(WorkspaceCleanAction::Proceed);
    }

    if !preconfirm {
        println!(
            "{} has {} window(s). Close all before launching the workset? [y/N]",
            label,
            state.candidates.len()
        );
        print!("> ");
        io::stdout().flush().context("failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read confirmation input")?;
        let proceed = matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if !proceed {
            return Ok(WorkspaceCleanAction::Cancelled);
        }
    }

    let expected_remaining = state.initial_clients.saturating_sub(state.candidates.len());
    let mut closed = 0usize;
    for c in state.candidates {
        if verbose {
            println!(
                " closing: {} ({})",
                c.class,
                if c.title.is_empty() {
                    "(no title)"
                } else {
                    c.title.as_str()
                }
            );
        }
        Dispatch::call(DispatchType::CloseWindow(WindowIdentifier::Address(
            c.address.clone(),
        )))
        .with_context(|| format!("failed to close window {}", c.address))?;
        closed += 1;
    }
    if closed > 0 {
        wait_for_clients_at_most(&state.context, expected_remaining, verbose)
            .with_context(|| format!("failed to wait for windows to close on {}", label))?;
    }
    println!("closed {closed} window(s) on {} before launch", label);
    Ok(WorkspaceCleanAction::Proceed)
}

fn collect_active_workspace_state(verbose: bool) -> Result<ActiveWorkspaceState> {
    let (context, clients) =
        resolve_active_workspace(verbose).context("failed to resolve active workspace context")?;
    let self_pid = std::process::id() as i32;
    let ancestors = collect_ancestor_pids()?;

    let mut candidates = Vec::new();
    let matching_clients = clients
        .iter()
        .filter(|c| context.matches(&c.workspace))
        .count();
    for c in clients
        .iter()
        .filter(|c| context.matches(&c.workspace) && c.pid != self_pid)
    {
        if ancestors.contains(&c.pid) {
            if verbose {
                println!(" skipping self/parent window: {} ({})", c.class, c.pid);
            }
            continue;
        }
        candidates.push(CloseCandidate {
            address: c.address.clone(),
            class: c.class.clone(),
            title: c.title.clone(),
        });
    }

    Ok(ActiveWorkspaceState {
        context,
        candidates,
        initial_clients: matching_clients,
    })
}

fn run_commands(ws: &Workset, verbose: bool) -> Result<()> {
    let cmds = &ws.commands;
    if cmds.is_empty() {
        println!("no commands to run");
        return Ok(());
    }

    for (idx, cmd) in cmds.iter().enumerate() {
        if verbose {
            println!(" exec[{idx}]: {cmd}");
        } else {
            println!(" exec: {cmd}");
        }
        let exec = build_exec_command(cmd, ws.cwd.as_deref(), [&ws.env]);
        Dispatch::call(DispatchType::Exec(exec.as_str()))
            .with_context(|| format!("failed to exec command: {cmd}"))?;
        if idx + 1 < cmds.len() {
            println!(" waiting {:?} before next command...", SLOT_LAUNCH_DELAY);
            thread::sleep(SLOT_LAUNCH_DELAY);
        }
    }
    Ok(())
}

fn run_layout(node: &LayoutNode, ws: &Workset, verbose: bool) -> Result<()> {
    let (workspace_ctx, current_clients) = resolve_active_workspace(verbose)
        .context("failed to determine active workspace for layout")?;
    let base_clients = current_clients
        .iter()
        .filter(|c| workspace_ctx.matches(&c.workspace))
        .count();
    let total_slots = count_slots(node);
    let mut launched = 0usize;
    let mut split_state = initial_split_state(verbose);
    let mut pending_ratio = None;
    run_layout_inner(
        node,
        ws,
        verbose,
        &mut launched,
        total_slots,
        &workspace_ctx,
        base_clients,
        &mut split_state,
        &mut pending_ratio,
    )
}

fn initial_split_state(verbose: bool) -> SplitDirection {
    let env_dir = env::var("HYPRSETS_INITIAL_SPLIT").ok();
    let from_env = env_dir
        .as_deref()
        .and_then(|v| match v.to_ascii_lowercase().as_str() {
            "h" | "horizontal" => Some(SplitDirection::Horizontal),
            "v" | "vertical" => Some(SplitDirection::Vertical),
            _ => None,
        });
    let dir = from_env.unwrap_or(SplitDirection::Horizontal);
    if verbose {
        let label = match dir {
            SplitDirection::Horizontal => "horizontal",
            SplitDirection::Vertical => "vertical",
        };
        if let Some(raw) = env_dir {
            println!(" initial split orientation from HYPRSETS_INITIAL_SPLIT={raw}: {label}");
        } else {
            println!(" initial split orientation defaulting to {label}");
        }
    }
    dir
}

#[allow(clippy::too_many_arguments)]
fn run_layout_inner(
    node: &LayoutNode,
    ws: &Workset,
    verbose: bool,
    launched: &mut usize,
    total_slots: usize,
    workspace: &WorkspaceContext,
    base_clients: usize,
    current_split: &mut SplitDirection,
    pending_ratio: &mut Option<f32>,
) -> Result<()> {
    match node {
        LayoutNode::Leaf(slot) => {
            let had_window = *launched > 0;
            println!(" slot #{} exec: {}", slot.slot_id, slot.command);
            let cwd = slot.cwd.as_deref().or(ws.cwd.as_deref());
            let exec = build_exec_command(&slot.command, cwd, [&ws.env, &slot.env]);
            Dispatch::call(DispatchType::Exec(exec.as_str()))
                .with_context(|| format!("failed to exec slot #{} command", slot.slot_id))?;
            *launched += 1;
            if had_window {
                flip_split_state(current_split);
            }
            let target_clients = base_clients + *launched;
            wait_for_clients_on_workspace(workspace, target_clients, verbose)?;

            if let Some(ratio) = pending_ratio.take() {
                apply_split_ratio(ratio, verbose);
            }

            let remaining = total_slots.saturating_sub(*launched);
            if remaining > 0 {
                println!(
                    " waiting {:?} before next slot... (remaining: {})",
                    SLOT_LAUNCH_DELAY, remaining
                );
                thread::sleep(SLOT_LAUNCH_DELAY);
            }
        }
        LayoutNode::Split(split) => {
            let hypr_ratio = to_hypr_split_ratio(split.ratio);
            if verbose {
                let dir = match split.direction {
                    crate::config::SplitDirection::Horizontal => "horizontal",
                    crate::config::SplitDirection::Vertical => "vertical",
                };
                println!(
                    " split {dir} ratio {:.2} (hyprctl {:.3})",
                    split.ratio, hypr_ratio
                );
            }
            run_layout_inner(
                &split.left,
                ws,
                verbose,
                launched,
                total_slots,
                workspace,
                base_clients,
                current_split,
                pending_ratio,
            )?;
            ensure_split_direction(split.direction, current_split, verbose)?;
            let mut right_pending = Some(hypr_ratio);
            run_layout_inner(
                &split.right,
                ws,
                verbose,
                launched,
                total_slots,
                workspace,
                base_clients,
                current_split,
                &mut right_pending,
            )?;

            let target_clients = base_clients + *launched;
            wait_for_clients_on_workspace(workspace, target_clients, verbose)?;
        }
    }
    Ok(())
}

fn to_hypr_split_ratio(user_ratio: f32) -> f32 {
    let safe = user_ratio.max(0.01);
    let converted = 2.0 * safe / (safe + 1.0);
    converted.clamp(HYPR_SPLIT_MIN, HYPR_SPLIT_MAX)
}

fn ensure_split_direction(
    desired: SplitDirection,
    current: &mut SplitDirection,
    verbose: bool,
) -> Result<()> {
    match (*current, desired) {
        (SplitDirection::Horizontal, SplitDirection::Horizontal)
        | (SplitDirection::Vertical, SplitDirection::Vertical) => {
            return Ok(());
        }
        _ => {}
    }

    let label = match desired {
        SplitDirection::Horizontal => "horizontal",
        SplitDirection::Vertical => "vertical",
    };
    if verbose {
        println!(" toggling split orientation -> {label}");
    }
    Dispatch::call(DispatchType::ToggleSplit)
        .context("failed to toggle split direction in Hyprland")?;
    *current = desired;
    Ok(())
}

fn apply_split_ratio(hypr_ratio: f32, verbose: bool) {
    let arg = format!("exact {:.3}", hypr_ratio);
    if verbose {
        println!(" applying split ratio: splitratio {arg}");
    }
    if let Err(err) = Dispatch::call(DispatchType::Custom("splitratio", arg.as_str())) {
        eprintln!(" split ratio change skipped (splitratio {arg}): {err}");
    }
}

fn flip_split_state(current: &mut SplitDirection) {
    *current = match *current {
        SplitDirection::Horizontal => SplitDirection::Vertical,
        SplitDirection::Vertical => SplitDirection::Horizontal,
    };
}

fn count_slots(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Leaf(_) => 1,
        LayoutNode::Split(split) => count_slots(&split.left) + count_slots(&split.right),
    }
}

fn wait_for_clients_on_workspace(
    workspace: &WorkspaceContext,
    target: usize,
    verbose: bool,
) -> Result<()> {
    let deadline = Instant::now() + WINDOW_APPEAR_TIMEOUT;
    let mut first_log = true;
    let label = workspace.label();
    loop {
        let clients = Clients::get().context("failed to list Hyprland clients")?;
        let count = clients
            .iter()
            .filter(|c| workspace.matches(&c.workspace))
            .count();

        if count >= target {
            if verbose {
                println!(" {}: clients ready ({}/{})", label, count, target);
            }
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!(
                "{}: timed out after {:?} waiting for clients ({}/{})",
                label,
                WINDOW_APPEAR_TIMEOUT,
                count,
                target
            );
        }

        if verbose && first_log {
            println!(" waiting for windows... {}: {}/{}", label, count, target);
            first_log = false;
        }
        thread::sleep(WINDOW_POLL_INTERVAL);
    }
}

fn wait_for_clients_at_most(workspace: &WorkspaceContext, max: usize, verbose: bool) -> Result<()> {
    let deadline = Instant::now() + WINDOW_APPEAR_TIMEOUT;
    let mut first_log = true;
    let label = workspace.label();
    loop {
        let clients = Clients::get().context("failed to list Hyprland clients")?;
        let count = clients
            .iter()
            .filter(|c| workspace.matches(&c.workspace))
            .count();
        if count <= max {
            if verbose {
                println!(" {}: clients drained ({}/{})", label, count, max);
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "{}: timed out after {:?} waiting for clients to close ({}/{})",
                label,
                WINDOW_APPEAR_TIMEOUT,
                count,
                max
            );
        }
        if verbose && first_log {
            println!(
                " waiting for windows to close... {}: {}/{}",
                label, count, max
            );
            first_log = false;
        }
        thread::sleep(WINDOW_POLL_INTERVAL);
    }
}

/// Collect ancestor PIDs (including self) so we don't close our own window while cleaning.
fn collect_ancestor_pids() -> Result<HashSet<i32>> {
    let mut pids = HashSet::new();
    let mut current = std::process::id() as i32;
    let mut hops = 0;

    loop {
        if !pids.insert(current) {
            break;
        }
        let status_path = format!("/proc/{current}/status");
        let Ok(status) = fs::read_to_string(&status_path) else {
            break;
        };

        let mut parent: Option<i32> = None;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("PPid:") {
                parent = rest.split_whitespace().next().and_then(|v| v.parse().ok());
                break;
            }
        }

        let Some(ppid) = parent else {
            break;
        };

        if ppid <= 1 {
            break;
        }

        current = ppid;
        hops += 1;
        if hops > 32 {
            break;
        }
    }

    Ok(pids)
}
