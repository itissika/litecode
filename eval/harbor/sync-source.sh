#!/usr/bin/env bash
# Move the eval source tree to the commit another checkout is on.
#
#   eval/harbor/sync-source.sh <source> [tree]
#
# <source> is either the Windows checkout as WSL sees it (e.g.
# /mnt/e/<drive>/litecode) or a bundle file written by
#   git -C <windows> bundle create head.bundle refs/heads/<branch>
# Both go through the same fetch, so a machine without /mnt access can copy a
# bundle over and point at it instead.
#
# The tree is a sparse checkout: a fetched commit brings its objects, and only
# the paths that are materialised change on disk.
set -euo pipefail

source=${1:?usage: sync-source.sh <git-repo-or-bundle> [tree]}
tree=${2:-"$HOME/build/litecode-src"}

if ! git -C "$tree" rev-parse --verify HEAD >/dev/null 2>&1; then
  echo "sync-source.sh: $tree is not a git checkout" >&2
  exit 1
fi

branch=$(git -C "$tree" rev-parse --abbrev-ref HEAD)
git -C "$tree" fetch --quiet "$source" "+refs/heads/$branch:refs/eval/source"
git -C "$tree" merge --ff-only FETCH_HEAD >/dev/null

echo "$tree is now at $(git -C "$tree" rev-parse HEAD) on $branch"
if [ -n "$(git -C "$tree" status --porcelain)" ]; then
  echo "sync-source.sh: warning: the working tree is not clean; build-info.json will say dirty" >&2
fi
