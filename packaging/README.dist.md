# HyprSets Distribution Contents

This directory contains the files bundled into the release tarball produced by `scripts/package.sh`.

## Included files
- `bin/hyprsets`: Release-built binary
- `share/applications/hyprsets.desktop`: Desktop entry; launches via Alacritty with `--class TUI.float` so Hyprland float rules are easy to apply
- `share/hyprsets/sample-worksets.toml`: Sample config for first-time setup
- `install.sh`: Installer script
- `CHECKSUMS.txt`: Checksums for files after extracting the tarball

## Usage (after extracting the tarball)
```
# System-wide install (root required)
sudo ./install.sh

# Install into the user home
./install.sh --user
```

### Options
- `--user`: Install into `~/.local/{bin,share/applications}` and create the sample config at `~/.config/hyprsets/hyprsets.toml`.
- `--prefix <path>`: Override the install prefix if you do not want `/usr/local`.
- `--no-config`: Skip extracting the sample config.
- `--force`: Overwrite existing config files (by default they are kept).

### Launching as floating on Hyprland
The `.desktop` entry runs `alacritty --class TUI.float -e <bin>`. Apply a class-based rule like `windowrulev2 = float,class:TUI.float` on the Hyprland side. If you prefer a different terminal, edit the `.desktop` file (either before packaging or after running `install.sh`).
