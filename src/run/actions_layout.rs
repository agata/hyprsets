use std::{thread, time::Instant};

use anyhow::{Context, Result, bail};
use hyprland::{
    data::Clients,
    dispatch::{Dispatch, DispatchType},
    shared::HyprData,
};

use crate::config::{LayoutNode, SplitDirection, Workset};

use super::{
    HYPR_SPLIT_MAX, HYPR_SPLIT_MIN, SLOT_LAUNCH_DELAY, WINDOW_APPEAR_TIMEOUT, WINDOW_POLL_INTERVAL,
    actions_workspace::{WorkspaceContext, WorkspaceTarget, ensure_workspace_focus},
    build_exec_command,
};

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
    let base_clients = current_clients
        .iter()
        .filter(|c| workspace.matches(&c.workspace))
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
        workspace,
        base_clients,
        &mut split_state,
        &mut pending_ratio,
        workspace_target,
    )
}

fn initial_split_state(verbose: bool) -> SplitDirection {
    let env_dir = std::env::var("HYPRSETS_INITIAL_SPLIT").ok();
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
    workspace_target: Option<&WorkspaceTarget>,
) -> Result<()> {
    match node {
        LayoutNode::Leaf(slot) => {
            ensure_workspace_focus(workspace_target, verbose)?;
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
                workspace_target,
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
                workspace_target,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypr_split_ratio_clamps_bounds() {
        assert!((to_hypr_split_ratio(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(to_hypr_split_ratio(0.0), HYPR_SPLIT_MIN);
        assert_eq!(to_hypr_split_ratio(100.0), HYPR_SPLIT_MAX);
    }

    #[test]
    fn flip_split_state_toggles() {
        let mut dir = SplitDirection::Horizontal;
        flip_split_state(&mut dir);
        assert!(matches!(dir, SplitDirection::Vertical));
        flip_split_state(&mut dir);
        assert!(matches!(dir, SplitDirection::Horizontal));
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
