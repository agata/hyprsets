# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]
### Added
- Per-slot launch delay override via `wait_after_ms` (defaults to 1s), editable from the layout editor slot dialog.

## [0.3.1] - 2025-12-08
### Changed
- Home tab bar now uses ratatui tabs with proper padding, hover/click hitboxes (including a clickable "+ Add" action), and aligned table spacing.
### Fixed
- Workset launch now keeps the target workspace focused while windows spawn, preventing apps from opening on the wrong workspace when the user switches during launch.

## [0.3.0] - 2025-12-08
### Added
- Tabbed workset management: single-tab membership, tab menu (reorder/rename/delete), assign dialog (`a`), tab-aware new/edit dialogs (default to current tab), and cleanup of stale tab references when worksets are removed.
- Refactored UI and runtime modules (home actions/events/render/tabs + run workspace/layout/lock/util) and added unit tests for workspace targets, split ratios, and command building utilities.

### Fixed
- Layout launch now preserves the configured split tree order: no unnecessary split toggles and correct focus anchoring so slots spawn in the intended panes.

## [0.2.1] - 2025-12-04
### Changed
- Workset list now supports multi-digit numeric selection (including `0` for the 10th entry); Enter launches the currently selected workset.

## [0.2.0] - 2025-12-03
### Added
- Configurable workspace targets per workset (including `special` scratchpads), plus a workspace column on the home list to show destinations.
- Launch lock and workspace focus handling so only one run proceeds at a time and the target workspace stays active while commands/layouts spawn.
- Added `hyprsets version` subcommand.
- UI improvements: workset ID editing, wrap-around navigation, and remembering the last selection after edits.
- Release automation pulls GitHub release descriptions from `CHANGELOG.md`.

### Changed
- Workset changes now auto-save during actions; exit-time bulk saves were removed.
- Reduced UI noise by removing ratio tweak toasts and waiting for workspace cleanup to settle before launching.

### Fixed
- Launching to scratchpad workspaces now works reliably, and cleanup targets the intended workspace.
- Cursor visibility in the new-workset dialog and launch lock file handling were corrected.

## [0.1.0] - 2025-12-03
### Added
- Initial release of HyprSets with a TUI home screen to run, duplicate, reorder, or delete worksets with keyboard and mouse.
- Layout editor that mirrors Hyprland-style splits, supports ratio tweaks, and edits slot commands in place.
- Launch workflows that traverse layouts or run sequential commands, with optional workspace targeting including special workspaces.
- Prompt to close existing windows on the active workspace before launching to keep layouts tidy.
- Config stored in TOML under `~/.config/hyprsets/hyprsets.toml`, created automatically with a sample.
