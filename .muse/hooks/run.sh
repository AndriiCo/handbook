#!/bin/sh
# Launcher for the project hooks in .muse/hooks.json. Muse runs hooks with a
# cleared environment, so HOME and PATH are set here; each hook is a small
# node script. /tmp never goes on PATH.
HOME=${HOME:-$(eval echo "~$(id -un)")}
export HOME
PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
case $HOME in
  /?*) [ -d "$HOME/.local/bin" ] && PATH="$PATH:$HOME/.local/bin" ;;
esac
export PATH
name=$1
shift
if [ "$name" = session-log ]; then
  # Every session, the judge's worktree included, writes its digest into the
  # MAIN checkout's docs/prompts/sessions, with the main checkout's script.
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
  cd "$(dirname "$common")" || exit 0
else
  cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" || exit 0
fi
exec node --no-warnings ".muse/hooks/$name.mjs" "$@"
