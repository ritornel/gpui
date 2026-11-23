#!/usr/bin/env bash
# Brings a new gpui-pre-linux release into this repository:
#
#   scripts/sync.sh 0.3.14
#
# 1. downloads the release from crates.io and checks it against the
#    checksum crates.io publishes for it;
# 2. commits it, unchanged, on `upstream`, tagged upstream/v0.3.14;
# 3. makes patched/v0.3.14 from that tag and cherry-picks onto it the
#    patch of the newest patched branch before it;
# 4. checks the patched files' formatting, runs kinetic.rs's tests, and
#    tags the result patched/v0.3.14-1.
#
# It works in temporary worktrees, so `main` stays checked out, and it
# pushes nothing: it ends by printing the push. Run again after a failure,
# it goes on from what the last run finished. A conflict stops it, with the
# worktree left to finish the cherry-pick in. It needs git, curl, jq, tar,
# sha256sum or shasum, rustc and rustfmt.
set -euo pipefail

version=${1:?usage: scripts/sync.sh <version>, such as 0.3.14}
crate=gpui-pre-linux
agent="ritornel-gpui-sync (https://github.com/ritornel/gpui)"
cd "$(git rev-parse --show-toplevel)"

work=$(mktemp -d)
# Whatever fails says where it stopped.
trap 'echo "Stopped at: $BASH_COMMAND. Nothing was pushed; run it again to go on." >&2' ERR

fail() {
    echo "$*" >&2
    exit 1
}

has() {
    git rev-parse --quiet --verify "$1" >/dev/null
}

# GitHub's branches into the remote-tracking ones. A local branch is made
# from each, or kept where an earlier run left it ahead.
git fetch --quiet --tags origin
for branch in $(git for-each-ref --format='%(refname:lstrip=3)' refs/remotes/origin/upstream 'refs/remotes/origin/patched/*'); do
    if has "refs/heads/$branch"; then
        git merge-base --is-ancestor "origin/$branch" "$branch" ||
            fail "$branch has moved apart from origin/$branch."
    else
        git branch --quiet "$branch" "origin/$branch"
    fi
done

previous=$(git tag --list 'upstream/v*' --sort=-v:refname | sed 's|^upstream/v||' | grep -vxF "$version" | head -n1 || true)
[[ -n $previous ]] || fail "There is no upstream/v* tag to sync from."
[[ $(printf '%s\n' "$previous" "$version" | sort -V | tail -n1) == "$version" ]] ||
    fail "$version is older than upstream/v$previous."
has "refs/heads/patched/v$previous" || fail "There is no patched/v$previous to take the patch from."

# 1 and 2. The release, checked before anything changes, then on
# `upstream` in place of the one before. A fresh worktree holds nothing but
# what the package brings.
if has "refs/tags/upstream/v$version"; then
    git merge-base --is-ancestor "upstream/v$version" upstream ||
        fail "upstream/v$version is not on upstream."
    echo "upstream/v$version: here already"
else
    checksum=$(curl -fsSL -A "$agent" "https://crates.io/api/v1/crates/$crate/$version" | jq -er .version.checksum)
    curl -fsSL -A "$agent" -o "$work/$crate.crate" "https://static.crates.io/crates/$crate/$crate-$version.crate"
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$work/$crate.crate")
    else
        actual=$(shasum -a 256 "$work/$crate.crate")
    fi
    [[ ${actual%% *} == "$checksum" ]] ||
        fail "$crate $version does not match the checksum crates.io gives, $checksum."
    git worktree add --quiet "$work/upstream" upstream
    (
        cd "$work/upstream"
        git rm -rq .
        tar xzf "$work/$crate.crate" --strip-components=1
        git add -A
        git commit --quiet -m "$crate $version, as published on crates.io" \
            -m "The package from https://crates.io/crates/$crate/$version unchanged (sha256 $checksum)."
        git tag -a -m "$crate $version as published" "upstream/v$version"
    )
    git worktree remove "$work/upstream"
    echo "upstream/v$version: $crate $version as published"
fi

# 3 and 4. The patch, onto the new release, checked, then tagged.
if has "refs/tags/patched/v$version-1"; then
    echo "patched/v$version-1: here already"
else
    # What an earlier run left of the branch, never pushed, is made again,
    # unless a cherry-pick is under way in it.
    git worktree prune
    stale=$(git worktree list --porcelain |
        awk -v branch="branch refs/heads/patched/v$version" '/^worktree /{tree = substr($0, 10)} $0 == branch {print tree}')
    if [[ -n $stale ]]; then
        if (cd "$stale" && [[ -e $(git rev-parse --git-path CHERRY_PICK_HEAD) ]]); then
            fail "A cherry-pick onto patched/v$version is under way in $stale. Finish it there (README.md, \"Syncing\"), or remove that worktree to start again."
        fi
        git worktree remove --force "$stale"
    fi
    if has "refs/heads/patched/v$version"; then
        has "refs/remotes/origin/patched/v$version" &&
            fail "patched/v$version is on GitHub without patched/v$version-1: check and tag it as README.md says (\"Syncing\")."
        git branch --quiet -D "patched/v$version"
    fi
    git worktree add --quiet -b "patched/v$version" "$work/patched" "upstream/v$version"
    cd "$work/patched"
    git cherry-pick "upstream/v$previous..patched/v$previous" ||
        fail "The patch does not apply cleanly to $version. Finish the cherry-pick in $work/patched, then check, tag and push as README.md says (\"Syncing\")."
    git diff --name-only -z "upstream/v$version" HEAD -- '*.rs' | xargs -0 rustfmt --edition 2024 --check
    rustc --edition 2024 --test -D warnings -o "$work/kinetic" src/linux/wayland/kinetic.rs
    "$work/kinetic" --quiet
    git tag -a -m "$crate $version with Ritornel's patch" "patched/v$version-1"
    cd - >/dev/null
    git worktree remove "$work/patched"
    echo "patched/v$version-1: the patch on $version, tested"
fi
rm -rf "$work"

echo
echo "To publish it, all at once:"
echo "  git push --atomic origin upstream patched/v$version upstream/v$version patched/v$version-1"
echo "  gh workflow run patched.yml"
echo "Then Ritornel moves gpui-pre and every gpui-pre-* pin to =$version, and the [patch] rev to $(git rev-parse "patched/v$version-1^{commit}")."
