use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    thread,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use hyprland::{
    data::{Client, Clients, Monitor, WorkspaceBasic},
    dispatch::{Dispatch, DispatchType, WindowIdentifier, WorkspaceIdentifierWithSpecial},
    shared::{Address, HyprData, HyprDataActive, HyprDataActiveOptional},
};

use crate::config::Workset;

use super::{
    WINDOW_APPEAR_TIMEOUT, WINDOW_POLL_INTERVAL, WORKSPACE_SWITCH_TIMEOUT, acquire_launch_lock,
    actions_layout::{run_commands, run_layout},
};

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
pub(crate) struct WorkspaceContext {
    pub workspace: WorkspaceBasic,
    pub is_special: bool,
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

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceTarget {
    kind: WorkspaceTargetKind,
}

#[derive(Clone, Debug)]
enum WorkspaceTargetKind {
    Id(i32),
    Name(String),
    Special(Option<String>),
}

impl WorkspaceContext {
    fn from_basic(workspace: WorkspaceBasic) -> Self {
        let is_special = workspace.id <= 0 || workspace.name.starts_with("special");
        Self {
            workspace,
            is_special,
        }
    }

    pub(crate) fn matches(&self, other: &WorkspaceBasic) -> bool {
        other.id == self.workspace.id
            || (!self.workspace.name.is_empty() && other.name == self.workspace.name)
    }

    pub(crate) fn label(&self) -> String {
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

impl WorkspaceTarget {
    fn from_raw(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(name) = trimmed.strip_prefix("special:") {
            return Self {
                kind: WorkspaceTargetKind::Special(Some(name.to_string())),
            };
        }
        if trimmed.eq_ignore_ascii_case("special") {
            return Self {
                kind: WorkspaceTargetKind::Special(None),
            };
        }
        if let Some(name) = trimmed.strip_prefix("name:") {
            return Self {
                kind: WorkspaceTargetKind::Name(name.to_string()),
            };
        }
        if let Ok(id) = trimmed.parse::<i32>() {
            return Self {
                kind: WorkspaceTargetKind::Id(id),
            };
        }

        Self {
            kind: WorkspaceTargetKind::Name(trimmed.to_string()),
        }
    }

    fn label(&self) -> String {
        match &self.kind {
            WorkspaceTargetKind::Id(id) => format!("workspace {id}"),
            WorkspaceTargetKind::Name(name) => format!("workspace '{name}'"),
            WorkspaceTargetKind::Special(None) => "special workspace".into(),
            WorkspaceTargetKind::Special(Some(name)) => {
                format!("special workspace '{name}'")
            }
        }
    }

    fn identifier(&self) -> WorkspaceIdentifierWithSpecial<'_> {
        match &self.kind {
            WorkspaceTargetKind::Id(id) => WorkspaceIdentifierWithSpecial::Id(*id),
            WorkspaceTargetKind::Name(name) => WorkspaceIdentifierWithSpecial::Name(name.as_str()),
            WorkspaceTargetKind::Special(None) => WorkspaceIdentifierWithSpecial::Special(None),
            WorkspaceTargetKind::Special(Some(name)) => {
                WorkspaceIdentifierWithSpecial::Special(Some(name.as_str()))
            }
        }
    }

    fn context(&self) -> WorkspaceContext {
        let basic = match &self.kind {
            WorkspaceTargetKind::Id(id) => WorkspaceBasic {
                id: *id,
                name: id.to_string(),
            },
            WorkspaceTargetKind::Name(name) => WorkspaceBasic {
                id: i32::MAX,
                name: name.clone(),
            },
            WorkspaceTargetKind::Special(None) => WorkspaceBasic {
                id: 0,
                name: "special".into(),
            },
            WorkspaceTargetKind::Special(Some(name)) => WorkspaceBasic {
                id: 0,
                name: format!("special:{name}"),
            },
        };
        WorkspaceContext::from_basic(basic)
    }

    fn matches(&self, ctx: &WorkspaceContext) -> bool {
        let target_ctx = self.context();
        ctx.matches(&target_ctx.workspace)
    }
}

fn workspace_override(ws: &Workset) -> Option<WorkspaceTarget> {
    ws.workspace
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(WorkspaceTarget::from_raw)
}

fn resolve_active_workspace(verbose: bool) -> Result<(WorkspaceContext, Clients)> {
    let clients = Clients::get().context("failed to list Hyprland clients")?;
    let active_client = Client::get_active().context("failed to get active window")?;
    if let Some(focused) = active_client {
        let ctx = WorkspaceContext::from_basic(focused.workspace);
        if verbose {
            println!(
                " active workspace determined from active window: {}",
                ctx.label()
            );
        }
        return Ok((ctx, clients));
    }

    let monitor = Monitor::get_active().context("failed to get active monitor from Hyprland")?;
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

fn ensure_target_active(target: &WorkspaceTarget, verbose: bool) -> Result<WorkspaceContext> {
    let (current_ctx, _) = resolve_active_workspace(false)?;
    if target.matches(&current_ctx) {
        if verbose {
            println!(" workspace override already active: {}", target.label());
        }
        return Ok(current_ctx);
    }

    if verbose {
        println!(" switching to {} before launch...", target.label());
    }
    Dispatch::call(DispatchType::Workspace(target.identifier()))
        .with_context(|| format!("failed to switch to {}", target.label()))?;

    wait_for_target_workspace(target, verbose)
}

pub(crate) fn ensure_workspace_focus(
    target: Option<&WorkspaceTarget>,
    verbose: bool,
) -> Result<()> {
    if let Some(target) = target {
        let (current_ctx, _) = resolve_active_workspace(false)?;
        if target.matches(&current_ctx) {
            return Ok(());
        }
        ensure_target_active(target, verbose)?;
    }
    Ok(())
}

pub(crate) fn resolve_launch_workspace(
    ws: &Workset,
    verbose: bool,
) -> Result<(Option<WorkspaceTarget>, WorkspaceContext)> {
    if let Some(target) = workspace_override(ws) {
        let ctx = ensure_target_active(&target, verbose)?;
        return Ok((Some(target), ctx));
    }

    let (ctx, _) = resolve_active_workspace(verbose)?;
    Ok((None, ctx))
}

fn wait_for_target_workspace(target: &WorkspaceTarget, verbose: bool) -> Result<WorkspaceContext> {
    let expected = target.context();
    let deadline = Instant::now() + WORKSPACE_SWITCH_TIMEOUT;
    loop {
        let (ctx, _) = resolve_active_workspace(false)?;
        if ctx.matches(&expected.workspace) {
            if verbose {
                println!(" active workspace is now {}", target.label());
            }
            return Ok(ctx);
        }
        if Instant::now() >= deadline {
            bail!(
                "timed out after {:?} waiting for {}",
                WORKSPACE_SWITCH_TIMEOUT,
                target.label()
            );
        }
        thread::sleep(WINDOW_POLL_INTERVAL);
    }
}

pub fn run_workset(ws: &Workset, verbose: bool, preconfirm_clean: bool) -> Result<()> {
    let _launch_lock = acquire_launch_lock(verbose)?;
    let (workspace_target, workspace_ctx) = resolve_launch_workspace(ws, verbose)?;

    match clean_workspace(&workspace_ctx, verbose, preconfirm_clean)
        .context("failed to clean target workspace before launch")?
    {
        WorkspaceCleanAction::Proceed => {}
        WorkspaceCleanAction::Cancelled => {
            println!("Launch cancelled: existing windows were kept open.");
            return Ok(());
        }
    }

    if let Some(layout) = &ws.layout {
        println!("launching workset '{}' with layout...", ws.name);
        run_layout(
            layout,
            ws,
            verbose,
            &workspace_ctx,
            workspace_target.as_ref(),
        )
        .with_context(|| format!("failed to launch layout (id: {})", ws.id))?;
    } else {
        println!("launching workset '{}' (commands sequential)...", ws.name);
        run_commands(ws, verbose, workspace_target.as_ref())
            .with_context(|| format!("failed to run commands (id: {})", ws.id))?;
    }
    Ok(())
}

pub fn workspace_cleanup_status(target: Option<&Workset>) -> Result<WorkspaceCleanupStatus> {
    let state = if let Some(ws) = target {
        if let Some(target_workspace) = workspace_override(ws) {
            let context = target_workspace.context();
            let clients = Clients::get().context("failed to list Hyprland clients")?;
            collect_workspace_state_with_clients(context, clients, false)?
        } else {
            collect_active_workspace_state(false)?
        }
    } else {
        collect_active_workspace_state(false)?
    };

    Ok(WorkspaceCleanupStatus {
        workspace_name: state.context.workspace.name.clone(),
        closable_windows: state.candidates.len(),
    })
}

fn clean_workspace(
    target_context: &WorkspaceContext,
    verbose: bool,
    preconfirm: bool,
) -> Result<WorkspaceCleanAction> {
    let state = collect_workspace_state(target_context.clone(), verbose)?;
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

fn collect_workspace_state(
    context: WorkspaceContext,
    verbose: bool,
) -> Result<ActiveWorkspaceState> {
    let clients = Clients::get().context("failed to list Hyprland clients")?;
    collect_workspace_state_with_clients(context, clients, verbose)
}

fn collect_active_workspace_state(verbose: bool) -> Result<ActiveWorkspaceState> {
    let (context, clients) =
        resolve_active_workspace(verbose).context("failed to resolve active workspace context")?;
    collect_workspace_state_with_clients(context, clients, verbose)
}

fn collect_workspace_state_with_clients(
    context: WorkspaceContext,
    clients: Clients,
    verbose: bool,
) -> Result<ActiveWorkspaceState> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx(id: i32, name: &str) -> WorkspaceContext {
        WorkspaceContext::from_basic(WorkspaceBasic {
            id,
            name: name.to_string(),
        })
    }

    #[test]
    fn workspace_target_from_raw_parses_variants() {
        let t1 = WorkspaceTarget::from_raw("special:foo");
        assert!(matches!(t1.kind, WorkspaceTargetKind::Special(Some(_))));
        let ctx_special = t1.context();
        assert!(ctx_special.is_special);
        assert!(t1.matches(&ctx_special));

        let t2 = WorkspaceTarget::from_raw("name:work");
        assert!(matches!(t2.kind, WorkspaceTargetKind::Name(_)));
        let ctx_named = ctx(1, "work");
        assert!(t2.matches(&ctx_named));

        let t3 = WorkspaceTarget::from_raw("3");
        assert!(matches!(t3.kind, WorkspaceTargetKind::Id(3)));
        let ctx_numeric = ctx(3, "");
        assert!(t3.matches(&ctx_numeric));
    }

    #[test]
    fn workspace_override_reads_workset_field() {
        let ws = Workset {
            id: "id".into(),
            name: "name".into(),
            desc: "".into(),
            workspace: Some("special:test".into()),
            commands: vec![],
            cwd: None,
            env: HashMap::new(),
            layout: None,
        };
        let target = workspace_override(&ws).expect("should parse override");
        assert!(matches!(target.kind, WorkspaceTargetKind::Special(Some(ref n)) if n == "test"));
    }

    #[test]
    fn workspace_context_label_includes_id_or_name() {
        let unnamed = ctx(4, "");
        assert!(unnamed.label().contains("4"));
        let named = ctx(2, "dev");
        assert!(named.label().contains("dev"));
        assert!(named.label().contains("2"));
    }

    #[test]
    fn workspace_context_matches_by_id_or_name() {
        let base = ctx(1, "alpha");
        assert!(base.matches(&WorkspaceBasic {
            id: 1,
            name: "".into(),
        }));
        assert!(base.matches(&WorkspaceBasic {
            id: 9,
            name: "alpha".into(),
        }));
        assert!(!base.matches(&WorkspaceBasic {
            id: 2,
            name: "beta".into(),
        }));
    }
}
