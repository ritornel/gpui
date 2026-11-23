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
# pushes nothing: it ends by printing the push. A conflict stops it, with
# the worktree left to finish the cherry-pick in. It needs git, curl, jq,
# tar, sha256sum or shasum, rustc and rustfmt.
set -euo pipefail

version=${1:?usage: scripts/sync.sh <version>, such as 0.3.14}
crate=gpui-pre-linux
agent="ritornel-gpui-sync (https://github.com/ritornel/gpui)"
cd "$(git rev-parse --show-toplevel)"

fail() {
    echo "$*" >&2
    exit 1
}

# Up to date with GitHub first: a branch that has moved apart stops it.
git fetch --quiet --tags origin upstream:upstream 'refs/heads/patched/*:refs/heads/patched/*'

previous=$(git tag --list 'upstream/v*' --sort=-v:refname | head -n1)
previous=${previous#upstream/v}
[[ -n $previous ]] || fail "There is no upstream/v* tag to sync from."
git rev-parse --quiet --verify "refs/tags/upstream/v$version" >/dev/null &&
    fail "upstream/v$version is here already."
[[ $(printf '%s\n' "$previous" "$version" | sort -V | tail -n1) == "$version" ]] ||
    fail "$version is older than upstream/v$previous."
git rev-parse --quiet --verify "refs/heads/patched/v$previous" >/dev/null ||
    fail "There is no patched/v$previous to take the patch from."

work=$(mktemp -d)
# Anything else that fails says where it stopped and what it left.
trap 'echo "Stopped at: $BASH_COMMAND. Work so far is in $work; nothing was pushed." >&2' ERR

# 1. The release, checked before anything changes.
checksum=$(curl -fsSL -A "$agent" "https://crates.io/api/v1/crates/$crate/$version" | jq -er .version.checksum)
curl -fsSL -A "$agent" -o "$work/$crate.crate" "https://static.crates.io/crates/$crate/$crate-$version.crate"
if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$work/$crate.crate")
else
    actual=$(shasum -a 256 "$work/$crate.crate")
fi
[[ ${actual%% *} == "$checksum" ]] || fail "$crate $version does not match the checksum crates.io gives, $checksum."

# 2. On `upstream`, in place of the release before: a fresh worktree holds
# nothing but what the package brings.
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

# 3. The patch, onto the new release.
git worktree add --quiet -b "patched/v$version" "$work/patched" "upstream/v$version"
cd "$work/patched"
git cherry-pick "upstream/v$previous..patched/v$previous" ||
    fail "The patch does not apply cleanly to $version. Finish the cherry-pick in $work/patched, then check, tag and push as README.md says (\"Syncing\")."

# 4. Checked, then tagged.
git diff --name-only -z "upstream/v$version" HEAD -- '*.rs' | xargs -0 rustfmt --edition 2024 --check
rustc --edition 2024 --test -D warnings -o "$work/kinetic" src/linux/wayland/kinetic.rs
"$work/kinetic" --quiet
git tag -a -m "$crate $version with Ritornel's patch" "patched/v$version-1"
cd - >/dev/null
git worktree remove "$work/patched"
rm -rf "$work"

echo "patched/v$version: the patch on $version, tested, tagged patched/v$version-1"
echo
echo "To publish it:"
echo "  git push origin upstream patched/v$version upstream/v$version patched/v$version-1"
echo "  gh workflow run patched.yml"
echo "Then Ritornel moves gpui-pre and every gpui-pre-* pin to =$version, and the [patch] rev to $(git rev-parse "patched/v$version-1^{commit}")."
