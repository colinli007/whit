#!/bin/bash
# Idle memory of Whit, Zed and VS Code right after opening a project, each
# with its out-of-the-box defaults. See docs/performance.md for the results.
#
#     scripts/memory_bench.sh <project> [whit|zed|vscode ...]
#
# Every app gets a brand-new empty profile (no user settings, extensions or
# restored session) and a Finder-like environment, and is measured alone. After
# SETTLE seconds the script samples SAMPLES times, INTERVAL seconds apart, and
# reports the median. The metric is phys_footprint, Activity Monitor's "Memory"
# column, summed over the app's whole process tree.
set -eu

SETTLE=${SETTLE:-90}
SAMPLES=${SAMPLES:-12}
INTERVAL=${INTERVAL:-10}
WHIT_APP=${WHIT_APP:-/Applications/Whit.app}
ZED_APP=${ZED_APP:-/Applications/Zed.app}
VSCODE_APP=${VSCODE_APP:-"/Applications/Visual Studio Code.app"}

if [ $# -lt 1 ] || [ ! -d "$1" ]; then
  sed -n '5p' "$0" | sed 's/^# *//' >&2
  exit 2
fi
PROJECT=$(cd "$1" && pwd)
shift
APPS=${*:-whit zed vscode}

# Under /tmp because VS Code's IPC socket path must stay within 103 characters.
PROFILES=$(mktemp -d /tmp/whit-bench.XXXXXX)
trap 'rm -rf "$PROFILES"' EXIT

descendants() {
  ps -A -o pid=,ppid= | awk -v root="$1" '
    { parent[$1] = $2; pids[NR] = $1 }
    END {
      keep[root] = 1; changed = 1
      while (changed) {
        changed = 0
        for (i in pids) { p = pids[i]; if (!(p in keep) && (parent[p] in keep)) { keep[p] = 1; changed = 1 } }
      }
      for (p in keep) print p
    }'
}

footprint_bytes() {
  footprint -f bytes --noCategories -p "$1" 2>/dev/null | awk '/phys_footprint:/ { print $2; exit }'
}

# "whit", "Code Helper (Renderer) --type=renderer", "zed --crash-handler", ...
process_label() {
  local args; args=$(ps -o args= -p "$1")
  printf '%s %s' "$(basename "$(ps -o comm= -p "$1")")" \
    "$(grep -oE -- '--(type|utility-sub-type)=[^ ]+|--crash-handler' <<<"$args" | tail -1)"
}

# "<bytes> <processes>" for the tree; with --list, one "<bytes> <label>" line per process.
measure() {
  local total=0 count=0 pid bytes
  for pid in $(descendants "$1"); do
    bytes=$(footprint_bytes "$pid")
    [ -n "$bytes" ] || continue
    if [ "${2:-}" = --list ]; then
      echo "$bytes $(process_label "$pid")"
    fi
    total=$((total + bytes))
    count=$((count + 1))
  done
  [ "${2:-}" = --list ] || echo "$total $count"
}

mb() { awk -v b="$1" 'BEGIN { printf "%.1f MB", b / 1048576 }'; }

app_version() { defaults read "$1/Contents/Info" CFBundleShortVersionString; }

# Runs a command the way Finder would: no environment from this shell.
launch() {
  local home=$1; shift
  env -i HOME="$home" USER="$USER" LOGNAME="$USER" SHELL=/bin/zsh TMPDIR="$TMPDIR" \
    PATH=/usr/bin:/bin:/usr/sbin:/sbin "$@" >/dev/null 2>&1 &
  ROOT=$!
  disown "$ROOT"
}

bench() {
  local name=$1 samples=() pid i
  echo "Measuring $name (pid $ROOT): settling ${SETTLE}s, then $SAMPLES samples every ${INTERVAL}s" >&2
  sleep "$SETTLE"
  # An app that hands the project to an instance already running, or fails to
  # start, exits here; measuring nothing would read as 0 MB.
  if ! kill -0 "$ROOT" 2>/dev/null; then
    echo "## $name"; echo; echo "The app exited during startup; quit any running instance and retry."; echo
    return
  fi
  for ((i = 0; i < SAMPLES; i++)); do
    samples+=("$(measure "$ROOT")")
    sleep "$INTERVAL"
  done

  echo "## $name"
  echo
  printf '%s\n' "${samples[@]}" | sort -n | awk -v n="$SAMPLES" '
    { bytes[NR] = $1; processes = $2 }
    END {
      printf "Median %.1f MB (min %.1f MB, max %.1f MB), %d process(es)\n\n",
        bytes[int((n + 1) / 2)] / 1048576, bytes[1] / 1048576, bytes[n] / 1048576, processes
    }'
  echo "| Process | Memory |"
  echo "|---|---|"
  measure "$ROOT" --list | sort -rn | while read -r bytes label; do
    echo "| $label | $(mb "$bytes") |"
  done
  echo

  for pid in $(descendants "$ROOT"); do kill -TERM "$pid" 2>/dev/null || true; done
  sleep 10
  for pid in $(descendants "$ROOT"); do kill -KILL "$pid" 2>/dev/null || true; done
}

echo "# Idle memory after opening $(basename "$PROJECT")"
echo
echo "$(date '+%Y-%m-%d %H:%M'), $(sysctl -n machdep.cpu.brand_string), $(($(sysctl -n hw.memsize) / 1073741824)) GB, macOS $(sw_vers -productVersion), swap: $(sysctl -n vm.swapusage)"
echo

for app in $APPS; do
  case $app in
    whit)
      mkdir -p "$PROFILES/whit-home"
      launch "$PROFILES/whit-home" "$WHIT_APP/Contents/MacOS/whit" "$PROJECT"
      bench "Whit $(app_version "$WHIT_APP")"
      ;;
    zed)
      launch "$HOME" "$ZED_APP/Contents/MacOS/zed" --user-data-dir "$PROFILES/zed" "$PROJECT"
      bench "Zed $(app_version "$ZED_APP")"
      ;;
    vscode)
      launch "$HOME" "$VSCODE_APP/Contents/MacOS/Code" \
        --user-data-dir "$PROFILES/vscode" --extensions-dir "$PROFILES/vscode-extensions" "$PROJECT"
      bench "VS Code $(app_version "$VSCODE_APP")"
      ;;
    *)
      echo "Unknown app: $app (expected whit, zed or vscode)" >&2
      exit 2
      ;;
  esac
done
