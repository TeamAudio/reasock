#!/usr/bin/env bash
# Builds ReaSock and symlinks it into REAPER's UserPlugins folder.
# REAPER only loads extensions at startup, so restart it after each build.
set -euo pipefail

cd "$(dirname "$0")/.."
profile="${1:-debug}"

if [[ "$profile" == "release" ]]; then
  cargo build --release
else
  cargo build
fi

case "$(uname -s)" in
  Darwin)
    src="$PWD/target/$profile/libreasock.dylib"
    dest="$HOME/Library/Application Support/REAPER/UserPlugins/reaper_reasock.dylib"
    ;;
  Linux)
    src="$PWD/target/$profile/libreasock.so"
    dest="$HOME/.config/REAPER/UserPlugins/reaper_reasock.so"
    ;;
  *)
    echo "unsupported platform: $(uname -s)" >&2
    exit 1
    ;;
esac

mkdir -p "$(dirname "$dest")"
ln -sf "$src" "$dest"
echo "linked $dest -> $src"
