#!/usr/bin/env bash
# Regenerates docs/screenshots/*.svg: every screen of the README and the
# usage guide, in English and Turkish, from the demo folder in demo.json.
#
#   python3 -m venv .venv && .venv/bin/pip install pyte   # once
#   cargo build --release
#   scripts/screenshots/make-shots.sh
#
# Review the images, then commit them with the change that altered a screen.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
BIN=${1:-$repo/target/release/rustclean}
PY=${PYTHON:-$repo/.venv/bin/python}
OUT=$repo/docs/screenshots

# A short path, so the title line shows everything after it.
work=$(mktemp -d /tmp/rustclean-shots.XXXX)
trap 'rm -rf "$work"' EXIT
python3 "$here/make-demo.py" "$work/demo"
DEMO=$work/demo

export ROWS=30 COLS=120 WAIT=0.6 START=3
export REPLACE="$DEMO=>~/demo"

down() { local s=""; for _ in $(seq "$1"); do s+="DOWN,"; done; echo -n "$s"; }
shot() { # name lang theme keys [env...]
  local name=$1 lang=$2 theme=$3 keys=$4; shift 4
  local data; data=$(mktemp -d)
  if [ -n "${LOGFILE:-}" ]; then cp "$LOGFILE" "$data/deletions.jsonl"; fi
  # `a`: apparent sizes, as the demo files are sparse.
  env RUSTCLEAN_DATA_DIR="$data" "$@" "$PY" "$here/svgshot.py" "$OUT/$name.svg" "a,$keys" \
    "$BIN" "$DEMO" --lang "$lang" --theme "$theme" >/dev/null
  rm -rf "$data"
}
both() { # name theme keys [env...]: English as name, Turkish as name-tr
  local name=$1 theme=$2 keys=$3; shift 3
  shot "$name" en "$theme" "$keys" "$@"
  shot "$name-tr" tr "$theme" "$keys" "$@"
}

both browser          dark ""
both treemap          dark "t,"
both summary          dark "i,"
both menu             dark "m,"
both dev-junk-stale   dark "m,$(down 5)ENTER,SLEEP1,f,f,"
both duplicates       dark "m,$(down 9)ENTER,SLEEP3,"
both basket           dark "m,ENTER,SLEEP1,SPACE,SPACE,S,"
both help             dark "QM,"
both clutter          dark "m,$(down 10)ENTER,SLEEP1,"
both device-backups   dark "m,$(down 11)ENTER,SLEEP1,"
both export           dark "o,"
both theme-light      light "" LIGHT=1
both theme-colorblind colorblind ""

# The deletion log, from a sample log.
now=$(date +%s)
LOGFILE=$work/deletions.jsonl
cat > "$LOGFILE" <<JSON
{"time":$((now-40*86400)),"path":"$HOME/demo/Movies/holiday-2023.mov","apparent":3221225472,"disk":3221225472,"via":"list","detail":""}
{"time":$((now-86400-3600*5)),"path":"$HOME/demo/Library/Caches/com.spotify.client","apparent":734003200,"disk":734003200,"via":"report","detail":"Caches"}
{"time":$((now-86400-3600*5)),"path":"$HOME/.npm/_cacache","apparent":1288490188,"disk":1288490188,"via":"tool","detail":"Npm"}
{"time":$((now-3600*3)),"path":"$HOME/demo/Applications/Sketchpad.app","apparent":157286400,"disk":157286400,"via":"uninstall","detail":"Sketchpad"}
{"time":$((now-3600*3)),"path":"$HOME/demo/Library/Application Support/com.example.sketchpad","apparent":57671680,"disk":57671680,"via":"uninstall","detail":"Sketchpad"}
{"time":$((now-3600*2)),"path":"$HOME/demo/Downloads/Xcode_16.xip","apparent":3758096384,"disk":3758096384,"via":"basket","detail":""}
{"time":$((now-3600*2)),"path":"$HOME/demo/Downloads/ubuntu-24.04-desktop-amd64.iso","apparent":6227702579,"disk":6227702579,"via":"basket","detail":""}
{"time":$((now-600)),"path":"$HOME/demo/Projects/web-app/node_modules","apparent":524288000,"disk":524288000,"via":"report","detail":"DevJunk"}
JSON
export LOGFILE
both deletion-log dark "m,$(down 16)ENTER,"
echo "wrote $(ls "$OUT"/*.svg | wc -l | tr -d ' ') screenshots to $OUT"
