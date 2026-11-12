#!/bin/sh
# Record the two plugins at work: Neovim and Emacs side by side in tmux,
# typed into by this script, captured by asciinema and rendered to a video.
#
#   echo 'v0/editors/demo.sh OUT_DIR' | pkg try asciinema asciinema-agg
#
# Needs tmux, nvim, emacs, ffmpeg, asciinema and agg on PATH, and a built
# v0 (V0_BIN, default: the release build). Writes OUT_DIR/v0-live.mp4.
#
# `demo.sh acp OUT_DIR` records the other demo instead: an ACP client (Emacs
# agent-shell) driving Claude Code onto a v0-only editing path, see
# acp/record.sh for its prerequisites (ADR-0018). Writes OUT_DIR/v0-acp-demo.mp4.
#
# The script is in two halves: called with OUT_DIR it sets the stage and
# records; called as `demo.sh --drive DIR` (by itself, in the background) it
# is the hands on the two keyboards.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
if [ "${1:-}" = acp ]; then shift; exec "$here/acp/record.sh" "$@"; fi
S=v0demo
# A tmux server of its own, with no config: the recording does not depend on
# whose machine it is made on.
tmux() { command tmux -L "$S" -f /dev/null "$@"; }

keys() { tmux send-keys -t "$S:0.$1" "$2"; }
# Type like a person: a character at a time.
type_() {
    pane=$1; text=$2
    while [ -n "$text" ]; do
        rest=${text#?}; c=${text%"$rest"}; text=$rest
        # A bare ";" ends a tmux command rather than being sent.
        [ "$c" = ";" ] && c='\;'
        tmux send-keys -t "$S:0.$pane" -l "$c"
        sleep 0.045
    done
}
say() { tmux set -t "$S" status-left " $1"; sleep "${2:-1.6}"; }
ex() { type_ 0 ":$1"; sleep 0.4; keys 0 Enter; }
mx() { keys 1 M-x; sleep 0.3; type_ 1 "$1"; sleep 0.4; keys 1 Enter; }

if [ "${1:-}" = --drive ]; then
    dir=$2
    sleep 1.5
    say "Neovim hosts a document: one replica of it, in a v0 live session" 1
    ex "V0Live $dir/s"
    sleep 1.2
    say "Emacs joins on the same socket, and is sent the text" 1
    mx v0-live-start
    sleep 0.3; keys 1 C-a; keys 1 C-k; type_ 1 "$dir/s"; sleep 0.4; keys 1 Enter
    sleep 1.8

    say "Typed in one, it appears in the other" 0.8
    keys 1 M-\>
    type_ 1 "// hello from emacs"; keys 1 Enter
    sleep 1
    keys 0 G
    type_ 0 "o// hello from neovim"; keys 0 Escape
    sleep 1.8

    say "Now both go offline" 0.8
    ex V0Hold
    mx v0-live-hold
    sleep 1

    say "Offline, Alice renames foo to bar: an intent, not a search and replace" 0.8
    type_ 0 "3G^"
    sleep 0.6
    ex "V0Rename bar"
    sleep 2

    say "Offline too, Bob adds a call to foo. He has not seen the rename" 0.8
    keys 1 M-\<; keys 1 C-n; keys 1 C-n; keys 1 C-n; keys 1 C-o
    type_ 1 "    foo();"
    sleep 2

    say "Both come back" 0.8
    ex V0Release
    sleep 0.6
    mx v0-live-release
    sleep 1
    say "Bob's call reads bar in both: the rename reached code it had never seen" 3
    tmux kill-session -t "$S"
    exit 0
fi

out=$(mkdir -p "$1" && cd "$1" && pwd)
bin=${V0_BIN:-$here/../target/release/v0}
# A Unix socket's path is short: the stage is under /tmp.
dir=$(mktemp -d /tmp/v0demo.XXXXXX)
mkdir "$dir/alice" "$dir/bob"
printf 'fn foo() {}\nfn main() {\n    foo();\n}\n' > "$dir/alice/demo.rs"

tmux kill-session -t "$S" 2>/dev/null || true
tmux new-session -d -s "$S" -x 132 -y 30 \
    "nvim -u NONE -i NONE -n --cmd 'set rtp+=$here/nvim' --cmd 'let g:v0_bin=\"$bin\"' \
        -c 'syntax on' -c 'set number laststatus=0 noshowmode' -c 'lua require(\"v0live\")' $dir/alice/demo.rs"
tmux split-window -h -t "$S" \
    "emacs -nw -Q --eval '(menu-bar-mode -1)' -l $here/emacs/v0-live.el --eval '(setq v0-live-bin \"$bin\")' $dir/bob/demo.rs"
tmux set -t "$S" default-terminal tmux-256color
tmux set -t "$S" escape-time 0
tmux set -t "$S" status-style "bg=colour236,fg=colour222"
tmux set -t "$S" status-left-length 132
tmux set -t "$S" status-right ""
tmux set -t "$S" window-status-current-format ""
tmux set -t "$S" pane-border-status top
tmux set -t "$S" pane-border-format " #{?#{==:#{pane_index},0},Alice: Neovim,Bob: Emacs} "
tmux set -t "$S" status-left " v0 live: two editors, one document"

"$0" --drive "$dir" &
TERM=xterm-256color asciinema rec --overwrite --headless --window-size 132x31 -f asciicast-v2 -c "tmux -L $S attach -t $S" "$out/v0-live.cast"
wait
# tmux leaves by clearing the screen and printing "[exited]": cut the cast
# where it leaves the alternate screen, so the last frame is the editors.
end=$(grep -n '?1049l' "$out/v0-live.cast" | tail -1 | cut -d: -f1)
head -n "$((end - 1))" "$out/v0-live.cast" > "$out/v0-live.cut" && mv "$out/v0-live.cut" "$out/v0-live.cast"
agg --quiet --font-size 18 --theme monokai --last-frame-duration 4 "$out/v0-live.cast" "$out/v0-live.gif"
ffmpeg -y -loglevel error -i "$out/v0-live.gif" -movflags faststart -pix_fmt yuv420p \
    -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" "$out/v0-live.mp4"
echo "$out/v0-live.mp4"
