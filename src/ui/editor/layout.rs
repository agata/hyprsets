use ratatui::layout::Rect;

use crate::config::{LayoutNode, SplitDirection, SplitNode, WindowSlot, Workset};

use super::{RATIO_MAX, RATIO_MIN, Side};

pub(super) fn ensure_layout(ws: Workset) -> LayoutNode {
    if let Some(layout) = ws.layout.clone() {
        layout
    } else {
        let cmd = ws.commands.first().cloned().unwrap_or_default();
        LayoutNode::Leaf(WindowSlot {
            slot_id: 1,
            command: cmd,
            cwd: None,
            env: Default::default(),
        })
    }
}

pub(super) fn split_area(area: Rect, dir: SplitDirection, ratio: f32) -> (Rect, Rect) {
    match dir {
        SplitDirection::Horizontal => {
            let total = area.width as f32;
            let left_w = ((ratio / (ratio + 1.0)) * total).max(1.0) as u16;
            let right_w = area.width.saturating_sub(left_w);
            let left = Rect {
                x: area.x,
                y: area.y,
                width: left_w,
                height: area.height,
            };
            let right = Rect {
                x: area.x + left_w,
                y: area.y,
                width: right_w,
                height: area.height,
            };
            (left, right)
        }
        SplitDirection::Vertical => {
            let total = area.height as f32;
            let top_h = ((ratio / (ratio + 1.0)) * total).max(1.0) as u16;
            let bottom_h = area.height.saturating_sub(top_h);
            let top = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: top_h,
            };
            let bottom = Rect {
                x: area.x,
                y: area.y + top_h,
                width: area.width,
                height: bottom_h,
            };
            (top, bottom)
        }
    }
}

pub(super) fn first_leaf_path(node: &LayoutNode) -> Option<Vec<Side>> {
    match node {
        LayoutNode::Leaf(_) => Some(vec![]),
        LayoutNode::Split(split) => {
            let mut left_path = vec![Side::Left];
            if let Some(rest) = first_leaf_path(&split.left) {
                left_path.extend(rest);
                return Some(left_path);
            }
            let mut right_path = vec![Side::Right];
            if let Some(rest) = first_leaf_path(&split.right) {
                right_path.extend(rest);
                return Some(right_path);
            }
            None
        }
    }
}

pub(super) fn next_slot_id(root: &LayoutNode) -> u32 {
    let mut max_id = 0;
    collect_slot_ids(root, &mut max_id);
    max_id + 1
}

pub(super) fn replace_leaf_with_split(
    node: &mut LayoutNode,
    path: &[Side],
    direction: SplitDirection,
    new_slot_id: u32,
) -> bool {
    if path.is_empty() {
        if let LayoutNode::Leaf(existing) = node {
            let new_leaf = LayoutNode::Leaf(WindowSlot {
                slot_id: new_slot_id,
                command: String::new(),
                cwd: None,
                env: Default::default(),
            });
            let old_leaf = LayoutNode::Leaf(existing.clone());
            let split = SplitNode {
                direction,
                ratio: if matches!(direction, SplitDirection::Horizontal) {
                    1.2
                } else {
                    1.0
                },
                left: Box::new(old_leaf),
                right: Box::new(new_leaf),
            };
            *node = LayoutNode::Split(split);
            return true;
        }
        return false;
    }

    match node {
        LayoutNode::Leaf(_) => false,
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first().unwrap();
            let child = if matches!(first, Side::Left) {
                &mut split.left
            } else {
                &mut split.right
            };
            replace_leaf_with_split(child, rest, direction, new_slot_id)
        }
    }
}

pub(super) fn remove_leaf(node: &mut LayoutNode, path: &[Side]) -> bool {
    if path.is_empty() {
        return false;
    }
    if let LayoutNode::Split(split) = node {
        let (first, rest) = path.split_first().unwrap();
        let target = if matches!(first, Side::Left) {
            &mut split.left
        } else {
            &mut split.right
        };

        if rest.is_empty() {
            let sibling = if matches!(first, Side::Left) {
                *split.right.clone()
            } else {
                *split.left.clone()
            };
            *node = sibling;
            return true;
        } else {
            return remove_leaf(target, rest);
        }
    }
    false
}

pub(super) fn collect_commands(node: &LayoutNode, commands: &mut Vec<String>) {
    match node {
        LayoutNode::Leaf(slot) => commands.push(slot.command.clone()),
        LayoutNode::Split(split) => {
            collect_commands(&split.left, commands);
            collect_commands(&split.right, commands);
        }
    }
}

pub(super) fn leaf_at_path<'a>(node: &'a LayoutNode, path: &[Side]) -> Option<&'a WindowSlot> {
    let mut cur = node;
    for side in path {
        match cur {
            LayoutNode::Split(split) => {
                cur = if matches!(side, Side::Left) {
                    &split.left
                } else {
                    &split.right
                };
            }
            LayoutNode::Leaf(_) => break,
        }
    }
    if let LayoutNode::Leaf(slot) = cur {
        Some(slot)
    } else {
        None
    }
}

pub(super) fn set_leaf_at_path(node: &mut LayoutNode, path: &[Side], slot: WindowSlot) -> bool {
    if path.is_empty() {
        if let LayoutNode::Leaf(target) = node {
            *target = slot;
            return true;
        }
        return false;
    }

    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first().unwrap();
            let child = if matches!(first, Side::Left) {
                &mut split.left
            } else {
                &mut split.right
            };
            set_leaf_at_path(child, rest, slot)
        }
        LayoutNode::Leaf(_) => false,
    }
}

pub(super) fn adjust_ratio(
    node: &mut LayoutNode,
    path: &[Side],
    delta: f32,
) -> Option<(SplitDirection, f32, f32)> {
    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first()?;
            if rest.is_empty() {
                let old = split.ratio;
                let new = clamp_ratio(old + delta);
                split.ratio = new;
                Some((split.direction, old, new))
            } else {
                let child = if matches!(first, Side::Left) {
                    &mut split.left
                } else {
                    &mut split.right
                };
                adjust_ratio(child, rest, delta)
            }
        }
        LayoutNode::Leaf(_) => None,
    }
}

pub(super) fn set_ratio(
    node: &mut LayoutNode,
    path: &[Side],
    new_ratio: f32,
) -> Option<(SplitDirection, f32, f32)> {
    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first()?;
            if rest.is_empty() {
                let old = split.ratio;
                let new = clamp_ratio(new_ratio);
                split.ratio = new;
                Some((split.direction, old, new))
            } else {
                let child = if matches!(first, Side::Left) {
                    &mut split.left
                } else {
                    &mut split.right
                };
                set_ratio(child, rest, new_ratio)
            }
        }
        LayoutNode::Leaf(_) => None,
    }
}

pub(super) fn ratio_from_position(
    area: Rect,
    direction: SplitDirection,
    x: u16,
    y: u16,
) -> Option<f32> {
    match direction {
        SplitDirection::Horizontal => {
            if area.width <= 1 {
                return None;
            }
            let pos = x
                .saturating_sub(area.x)
                .min(area.width.saturating_sub(1))
                .max(1);
            let left = pos as f32;
            let right = (area.width as f32 - left).max(1.0);
            Some(clamp_ratio(left / right))
        }
        SplitDirection::Vertical => {
            if area.height <= 1 {
                return None;
            }
            let pos = y
                .saturating_sub(area.y)
                .min(area.height.saturating_sub(1))
                .max(1);
            let top = pos as f32;
            let bottom = (area.height as f32 - top).max(1.0);
            Some(clamp_ratio(top / bottom))
        }
    }
}

pub(super) fn clamp_ratio(val: f32) -> f32 {
    val.clamp(RATIO_MIN, RATIO_MAX)
}

fn collect_slot_ids(node: &LayoutNode, max_id: &mut u32) {
    match node {
        LayoutNode::Leaf(slot) => {
            *max_id = (*max_id).max(slot.slot_id);
        }
        LayoutNode::Split(split) => {
            collect_slot_ids(&split.left, max_id);
            collect_slot_ids(&split.right, max_id);
        }
    }
}
