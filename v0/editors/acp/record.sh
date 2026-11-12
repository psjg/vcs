#!/bin/sh
# Record an ACP client driving Claude Code onto a v0-only editing path:
# Emacs with agent-shell on the left, what v0 sees on the right, captured by
# asciinema and rendered to an mp4.  Called by `demo.sh acp OUT_DIR`.
#
# Needs on PATH: node, git, jq, tmux, emacs (with network once, for
# agent-shell), asciinema, agg, ffmpeg; and the ACP adapter, which
# V0ACP_CMD names (default: claude-agent-acp on PATH).  The agent is real,
# the "v0" is the one-tool stub mcp-stub.mjs: the demo shows the guard
# (built-in writers denied, v0 offered), not the engine.  Writes
# OUT_DIR/v0-acp-demo.{cast,gif,mp4}.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(mkdir -p "$1" && cd "$1" && pwd)
S=v0acpdemo
tm() { command tmux -L "$S" -f /dev/null "$@"; }

V0ACP_CMD=${V0ACP_CMD:-$(command -v claude-agent-acp || true)}
[ -n "$V0ACP_CMD" ] || { echo "record.sh: set V0ACP_CMD to the claude-agent-acp binary" >&2; exit 2; }
for t in node git jq tmux emacs asciinema agg ffmpeg; do
    command -v "$t" >/dev/null || { echo "record.sh: $t not on PATH" >&2; exit 2; }
done

# The stage: a fresh repo that denies the built-in writers and offers v0.
stage=$(mktemp -d /tmp/v0acp.XXXXXX)
repo=$stage/repo; log=$stage/stub.log
mkdir -p "$repo/.claude"; : > "$log"
git -C "$repo" init -q
printf 'hello\n' > "$repo/a.txt"
printf '{"enabledMcpjsonServers":["v0"],"permissions":{"deny":["Edit","Write","MultiEdit","NotebookEdit","Bash"]}}\n' > "$repo/.claude/settings.json"
printf '{"mcpServers":{"v0":{"command":"node","args":["%s"],"env":{"STUB_LOG":"%s"}}}}\n' \
    "$here/mcp-stub.mjs" "$log" > "$repo/.mcp.json"

V0ACP_ELPA=${V0ACP_ELPA:-$out/elpa}
if ! ls -d "$V0ACP_ELPA"/agent-shell-* >/dev/null 2>&1; then
    V0ACP_ELPA=$V0ACP_ELPA emacs --batch -Q -l "$here/install.el"
fi
export V0ACP_ELPA V0ACP_CMD V0ACP_REPO=$repo V0ACP_LOG=$log

tm kill-server 2>/dev/null || true
tm new-session -d -s "$S" -x 150 -y 40 \
    "TERM=xterm-256color emacs -nw -Q -l $here/drive.el; tmux -L $S kill-server"
tm set -g status off
tm split-window -h -l 58 -t "$S" "sh $here/side.sh $repo $log"
tm select-pane -t "$S:0.0"
TERM=xterm-256color asciinema rec --overwrite --headless --window-size 150x40 \
    -f asciicast-v2 -c "tmux -L $S attach -t $S" "$out/v0-acp-demo.cast"
# tmux leaves by clearing the screen and printing "[server exited]": cut the
# cast where it leaves the alternate screen, so the last frame is the work.
end=$(grep -n '?1049l' "$out/v0-acp-demo.cast" | tail -1 | cut -d: -f1 || true)
if [ -n "$end" ]; then
    head -n "$((end - 1))" "$out/v0-acp-demo.cast" > "$out/cut.tmp" && mv "$out/cut.tmp" "$out/v0-acp-demo.cast"
fi
agg --quiet --font-size 16 --theme monokai --idle-time-limit 2 --last-frame-duration 3 \
    "$out/v0-acp-demo.cast" "$out/v0-acp-demo.gif"
ffmpeg -y -loglevel error -i "$out/v0-acp-demo.gif" -movflags faststart -pix_fmt yuv420p \
    -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" "$out/v0-acp-demo.mp4"
echo "$out/v0-acp-demo.mp4"
