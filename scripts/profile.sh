#!/usr/bin/env bash
# Record a CPU profile of an `lx` command with `samply` and open it in the
# Firefox Profiler.
#
# Usage:
#   scripts/profile.sh <subcommands>
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v samply >/dev/null 2>&1; then
  echo "error: samply not found. Please enter the devshell." >&2
  exit 1
fi

if [ -r /proc/sys/kernel/perf_event_paranoid ]; then
  paranoid="$(cat /proc/sys/kernel/perf_event_paranoid)"
  if [ "$paranoid" -gt 1 ]; then
    echo "error: kernel.perf_event_paranoid=$paranoid blocks samply." >&2
    echo "run: sudo sysctl kernel.perf_event_paranoid=1" >&2
    exit 1
  fi
fi

cargo build --profile profiling -p lux-cli --bin lx

[ "$#" -gt 0 ] || set -- install busted --force

exec samply record --profile-name "lx $*" -- "$root/target/profiling/lx" "$@"
