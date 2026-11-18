#!/bin/sh
# Prints the install line exactly as the README shows it, so the install tests run what people
# actually paste (SQ-47). With a second argument, the address of the script is replaced by a
# local server holding a build under test; the rest of the line stays as written.
#
#   sh readme-command.sh linux|windows [http://127.0.0.1:8017]
set -eu
cd "$(dirname "$0")/../.."
case "$1" in
  linux) LINE=$(grep -m1 '^{ curl .*tools/get\.sh.*| sh$' README.md) ;;
  windows) LINE=$(grep -m1 '^irm .*tools/get\.ps1 | iex$' README.md) ;;
  *) echo "usage: readme-command.sh linux|windows [LOCAL_SERVER]" >&2 && exit 2 ;;
esac
[ -n "$LINE" ] || { echo "the README has no install line for $1" >&2; exit 1; }
if [ $# -gt 1 ]; then
  LINE=$(printf '%s' "$LINE" | sed "s#https://raw.githubusercontent.com/yujideluca/status-quo/main/tools#$2#g")
fi
printf '%s\n' "$LINE"
