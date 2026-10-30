#!/bin/sh
# Launcher for the project hooks in .muse/hooks.json. Muse runs hooks with a
# cleared environment, so PATH is set here; each hook is a small node script.
PATH="/opt/homebrew/bin:/usr/local/bin:${HOME:-/tmp}/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
export PATH
cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" || exit 0
name=$1
shift
exec node --no-warnings ".muse/hooks/$name.mjs" "$@"
