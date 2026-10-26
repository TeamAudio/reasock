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

python3 scripts/build_docs.py

case "$(uname -s)" in
  Darwin)
    resource="$HOME/Library/Application Support/REAPER"
    src="$PWD/target/$profile/libreasock.dylib"
    dest="$resource/UserPlugins/reaper_reasock.dylib"
    ;;
  Linux)
    resource="$HOME/.config/REAPER"
    src="$PWD/target/$profile/libreasock.so"
    dest="$resource/UserPlugins/reaper_reasock.so"
    ;;
  *)
    echo "unsupported platform: $(uname -s)" >&2
    exit 1
    ;;
esac

mkdir -p "$(dirname "$dest")"
ln -sf "$src" "$dest"
echo "linked $dest -> $src"

# The API reference, where ReaPack would install it. Leave a real install alone.
docs="$resource/Data/reasock"
if [[ -L "$docs" || ! -e "$docs" ]]; then
  mkdir -p "$resource/Data"
  ln -sfn "$PWD/target/docs" "$docs"
  echo "linked $docs -> $PWD/target/docs"
else
  echo "left $docs alone (not a symlink)"
fi
