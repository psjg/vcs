#!/bin/sh
# The right pane of the ACP demo: what v0 sees. Redraws the repo's guard
# files, a.txt and every call the v0 stub received, once a second.
#   side.sh REPO CALL_LOG
R=$1; L=$2
while :; do
  out=$(
  printf '\033[1mv0 demo: an agent edits only through v0\033[0m\n\n'
  printf '\033[33m.claude/settings.json\033[0m  (built-in writers off)\n'; jq -c .permissions.deny "$R/.claude/settings.json"
  printf '\n\033[33m.mcp.json\033[0m  (v0 offered as the editing tool)\n'; jq -r '.mcpServers|keys[]' "$R/.mcp.json"
  printf '\n\033[33ma.txt\033[0m\n'; cat "$R/a.txt"
  printf '\n\033[33mcalls v0 received\033[0m\n'
  if [ -s "$L" ]; then jq -r '.arguments | "replace \(.old|@json) -> \(.new|@json)\n  reason: \(.reason)"' "$L"; else echo '(none yet)'; fi)
  clear; printf '%s\n' "$out"; sleep 1
done
