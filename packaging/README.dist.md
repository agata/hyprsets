# HyprSets 配布物について

このディレクトリは `scripts/package.sh` で作る配布用 tarball に同梱されるファイル一式です。

## 同梱物
- `bin/hyprsets`: リリースビルド済みバイナリ
- `share/applications/hyprsets.desktop`: デスクトップエントリ（デフォルトで Alacritty `--class TUI.float` に乗せて起動し、Hyprland 側で float ルールを貼りやすくしてあります）
- `share/hyprsets/sample-worksets.toml`: サンプル設定（初回作成用）
- `install.sh`: インストール用スクリプト
- `CHECKSUMS.txt`: tarball 展開後のファイルチェックサム

## 使い方（tarball 展開後）
```
# システム全体に入れる例（root が必要）
sudo ./install.sh

# ユーザホームに入れる例
./install.sh --user
```

### オプション
- `--user` : `~/.local/{bin,share/applications}` に配置し、サンプル設定を `~/.config/hyprsets/hyprsets.toml` に作成します。
- `--prefix <path>` : `/usr/local` 以外へ入れたい場合のパス上書き。
- `--no-config` : サンプル設定の展開をスキップ。
- `--force` : 既存の設定ファイルを上書きします（通常は既存があれば残します）。

### Hyprland で float 起動させる場合
`.desktop` の Exec は `alacritty --class TUI.float -e <bin>` になっています。Hyprland の `windowrulev2 = float,class:TUI.float` のようにクラス指定で浮かせてください。Alacritty 以外を使いたい場合は `.desktop` を書き換えてください（install.sh 実行後に編集しても可）。
