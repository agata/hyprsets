use std::{thread, time::Instant};

use anyhow::{Context, Result, bail};
use hyprland::{
    data::{Client, Clients},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{Address, HyprData, HyprDataActiveOptional},
};

use crate::config::{LayoutNode, Workset};

use super::{
    HYPR_SPLIT_MAX, HYPR_SPLIT_MIN, SLOT_LAUNCH_DELAY, WINDOW_APPEAR_TIMEOUT, WINDOW_POLL_INTERVAL,
    actions_workspace::{WorkspaceContext, WorkspaceTarget, ensure_workspace_focus},
    build_exec_command,
};
use std::collections::HashSet;

pub(crate) fn run_commands(
    ws: &Workset,
    verbose: bool,
    workspace_target: Option<&WorkspaceTarget>,
) -> Result<()> {
    let cmds = &ws.commands;
    if cmds.is_empty() {
        println!("no commands to run");
        return Ok(());
    }

    for (idx, cmd) in cmds.iter().enumerate() {
        ensure_workspace_focus(workspace_target, verbose)?;
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_layout(
    node: &LayoutNode,
    ws: &Workset,
    verbose: bool,
    workspace: &WorkspaceContext,
    workspace_target: Option<&WorkspaceTarget>,
) -> Result<()> {
    let current_clients = Clients::get().context("failed to list Hyprland clients")?;
    let mut known_clients: HashSet<Address> = current_clients
        .iter()
        .filter(|c| workspace.matches(&c.workspace))
        .map(|c| c.address.clone())
        .collect();
    let base_clients = known_clients.len();
    let total_slots = count_slots(node);
    let mut launched = 0usize;
    let mut launched_slots = HashSet::new();
    let mut pending_ratio = None;
    run_layout_inner(
        node,
        ws,
        verbose,
        &mut launched,
        total_slots,
        workspace,
        base_clients,
        &mut pending_ratio,
        workspace_target,
        &mut known_clients,
        &mut launched_slots,
    )?;
    Ok(())
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
    pending_ratio: &mut Option<f32>,
    workspace_target: Option<&WorkspaceTarget>,
    known_clients: &mut HashSet<Address>,
    launched_slots: &mut HashSet<u32>,
) -> Result<Option<Address>> {
    match node {
        LayoutNode::Leaf(slot) => {
            if !launched_slots.insert(slot.slot_id) {
                return Ok(active_address_on_workspace(workspace));
            }
            ensure_workspace_focus(workspace_target, verbose)?;
            println!(" slot #{} exec: {}", slot.slot_id, slot.command);
            let cwd = slot.cwd.as_deref().or(ws.cwd.as_deref());
            let exec = build_exec_command(&slot.command, cwd, [&ws.env, &slot.env]);
            Dispatch::call(DispatchType::Exec(exec.as_str()))
                .with_context(|| format!("failed to exec slot #{} command", slot.slot_id))?;
            *launched += 1;
            let target_clients = base_clients + *launched;
            wait_for_clients_on_workspace(workspace, target_clients, verbose)?;

            if let Some(ratio) = pending_ratio.take() {
                apply_split_ratio(ratio, verbose);
            }

            let anchor = newly_added_address(workspace, known_clients)
                .or_else(|| active_address_on_workspace(workspace));
            if let Some(addr) = &anchor {
                known_clients.insert(addr.clone());
            }

            let remaining = total_slots.saturating_sub(*launched);
            if remaining > 0 {
                println!(
                    " waiting {:?} before next slot... (remaining: {})",
                    SLOT_LAUNCH_DELAY, remaining
                );
                thread::sleep(SLOT_LAUNCH_DELAY);
            }
            Ok(anchor)
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
            let (left_anchor, left_was_split) = match &*split.left {
                LayoutNode::Split(_) => {
                    let anchor = run_left_anchor(
                        &split.left,
                        ws,
                        verbose,
                        launched,
                        total_slots,
                        workspace,
                        base_clients,
                        pending_ratio,
                        workspace_target,
                        known_clients,
                        launched_slots,
                    )?;
                    (anchor, true)
                }
                _ => {
                    let anchor = run_layout_inner(
                        &split.left,
                        ws,
                        verbose,
                        launched,
                        total_slots,
                        workspace,
                        base_clients,
                        pending_ratio,
                        workspace_target,
                        known_clients,
                        launched_slots,
                    )?;
                    (anchor, false)
                }
            };
            if let Some(addr) = left_anchor.clone() {
                focus_window(addr, workspace, verbose, workspace_target)?;
            } else {
                ensure_workspace_focus(workspace_target, verbose)?;
            }
            let mut right_pending = Some(hypr_ratio);
            let right_anchor = run_layout_inner(
                &split.right,
                ws,
                verbose,
                launched,
                total_slots,
                workspace,
                base_clients,
                &mut right_pending,
                workspace_target,
                known_clients,
                launched_slots,
            )?;

            let target_clients = base_clients + *launched;
            wait_for_clients_on_workspace(workspace, target_clients, verbose)?;
            let remaining_left = if left_was_split {
                if let Some(addr) = left_anchor.clone() {
                    focus_window(addr, workspace, verbose, workspace_target)?;
                } else {
                    ensure_workspace_focus(workspace_target, verbose)?;
                }
                run_layout_inner(
                    &split.left,
                    ws,
                    verbose,
                    launched,
                    total_slots,
                    workspace,
                    base_clients,
                    pending_ratio,
                    workspace_target,
                    known_clients,
                    launched_slots,
                )?
            } else {
                None
            };
            Ok(left_anchor.or(right_anchor).or(remaining_left))
        }
    }
}

fn to_hypr_split_ratio(user_ratio: f32) -> f32 {
    let safe = user_ratio.max(0.01);
    let converted = 2.0 * safe / (safe + 1.0);
    converted.clamp(HYPR_SPLIT_MIN, HYPR_SPLIT_MAX)
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

fn count_slots(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Leaf(_) => 1,
        LayoutNode::Split(split) => count_slots(&split.left) + count_slots(&split.right),
    }
}

fn focus_window(
    addr: Address,
    workspace: &WorkspaceContext,
    verbose: bool,
    workspace_target: Option<&WorkspaceTarget>,
) -> Result<()> {
    let id = WindowIdentifier::Address(addr.clone());
    if verbose {
        println!(" focusing window at address {addr}");
    }
    Dispatch::call(DispatchType::FocusWindow(id))
        .with_context(|| format!("failed to focus window at address {addr}"))?;
    // keep workspace focus stable even if focuswindow was ignored
    ensure_workspace_focus(workspace_target, verbose)?;
    let active = Client::get_active()
        .context("failed to get active window after focusing")?
        .filter(|c| workspace.matches(&c.workspace))
        .map(|c| c.address);
    if active != Some(addr.clone()) && verbose {
        println!(" focus verification skipped/failed (expected {addr:?}, got {active:?})");
    }
    Ok(())
}

fn active_address_on_workspace(workspace: &WorkspaceContext) -> Option<Address> {
    Client::get_active()
        .ok()
        .flatten()
        .filter(|c| workspace.matches(&c.workspace))
        .map(|c| c.address)
}

fn newly_added_address(workspace: &WorkspaceContext, known: &HashSet<Address>) -> Option<Address> {
    Clients::get().ok().and_then(|clients| {
        clients
            .into_iter()
            .filter(|c| workspace.matches(&c.workspace))
            .map(|c| c.address)
            .find(|addr| !known.contains(addr))
    })
}

#[allow(clippy::too_many_arguments)]
fn run_left_anchor(
    node: &LayoutNode,
    ws: &Workset,
    verbose: bool,
    launched: &mut usize,
    total_slots: usize,
    workspace: &WorkspaceContext,
    base_clients: usize,
    pending_ratio: &mut Option<f32>,
    workspace_target: Option<&WorkspaceTarget>,
    known_clients: &mut HashSet<Address>,
    launched_slots: &mut HashSet<u32>,
) -> Result<Option<Address>> {
    match node {
        LayoutNode::Leaf(_) => run_layout_inner(
            node,
            ws,
            verbose,
            launched,
            total_slots,
            workspace,
            base_clients,
            pending_ratio,
            workspace_target,
            known_clients,
            launched_slots,
        ),
        LayoutNode::Split(split) => run_left_anchor(
            &split.left,
            ws,
            verbose,
            launched,
            total_slots,
            workspace,
            base_clients,
            pending_ratio,
            workspace_target,
            known_clients,
            launched_slots,
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SplitDirection;

    #[test]
    fn hypr_split_ratio_clamps_bounds() {
        assert!((to_hypr_split_ratio(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(to_hypr_split_ratio(0.0), HYPR_SPLIT_MIN);
        assert_eq!(to_hypr_split_ratio(100.0), HYPR_SPLIT_MAX);
    }

    #[test]
    fn count_slots_calculates_total() {
        let layout = LayoutNode::Split(crate::config::SplitNode {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            left: Box::new(LayoutNode::Leaf(crate::config::WindowSlot {
                slot_id: 1,
                command: "a".into(),
                cwd: None,
                env: Default::default(),
            })),
            right: Box::new(LayoutNode::Split(crate::config::SplitNode {
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                left: Box::new(LayoutNode::Leaf(crate::config::WindowSlot {
                    slot_id: 2,
                    command: "b".into(),
                    cwd: None,
                    env: Default::default(),
                })),
                right: Box::new(LayoutNode::Leaf(crate::config::WindowSlot {
                    slot_id: 3,
                    command: "c".into(),
                    cwd: None,
                    env: Default::default(),
                })),
            })),
        });
        assert_eq!(count_slots(&layout), 3);
    }
}
