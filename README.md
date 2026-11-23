# gpui-pre-linux, patched for Ritornel

[Ritornel](https://github.com/ritornel/ritornel)'s patched copy of the [`gpui-pre-linux`](https://crates.io/crates/gpui-pre-linux) crate, the weekly crates.io snapshot of the `gpui_linux` crate in [Zed](https://github.com/zed-industries/zed)'s in-tree GPUI, under the Apache 2.0 licence that comes with each release (`LICENSE-APACHE` there). Ritornel's desktop app uses a patched branch in place of the published crate, with `[patch.crates-io]`.

This branch, `main`, holds only this README, the sync script and CI. The crate is on the other branches.

## The patch

GPUI's Wayland backend ignores a touchpad's `axis_stop`, so its scroll events carry no phase and the page stops dead when the fingers lift ([zed-industries/zed#14473](https://github.com/zed-industries/zed/issues/14473)). The patch changes that, and is meant to go upstream:

- A frame's scroll is a touchpad's when that frame's `axis_source` says so. It is `TouchPhase::Started`, then `Moved`, then `Ended` with no movement when the fingers lift, as on macOS. They have lifted once every axis they scrolled has had its `axis_stop`: one axis may stop while the other goes on.
- After the lift the page coasts (`src/linux/wayland/kinetic.rs`), one `Moved` scroll event a frame, where the pointer is, as macOS sends momentum. The window draws while the coast goes on.
- The coast follows the numbers GTK 4 (4.20) uses in `GtkScrolledWindow`: the speed over the last 150 ms before the lift, slowing at a rate of 4 a second until below 0.1 px/s. A flick that catches a coast, the same way and at least a third as fast, adds up to six times that coast's speed. `kinetic.rs` is written from GTK's behaviour, not from its code. Distances stay GPUI's, 3 pixels for each unit a touchpad scrolls where GTK moves 2.5, so a flick goes as far, for the scroll that threw it, as in GTK.
- Fingers resting on the touchpad (the pointer gestures protocol's hold gesture, from its version 3) or a new scroll catch a coast, and fingers that lift without scrolling let it go. A click or a pinch ends it. The pointer leaving, the window under it or coasting closing, or a new pointer forget the scroll altogether.

The patch starts as two commits on each patched branch, `kinetic.rs` and then the changes to the published files, each marked `// Ritornel:`. Fixes follow as commits of their own.

## Branches and tags

| Branch | What it holds | Tags |
|---|---|---|
| `main` | This README, `scripts/sync.sh` and CI | |
| `upstream` | Each published release, unchanged from its `.crate`, a commit for each with its sha256 in the message. Only ever appended to. | `upstream/v0.3.13`, … |
| `patched/v0.3.13`, … | A release's `upstream` tag plus the patch | `patched/v0.3.13-1`, … |

A patched branch is never rewritten once Ritornel pins it. A newer release gets a new `patched/v<version>` branch, and the old ones stay as they are, so an old pin always resolves. Ritornel pins the commit of a `patched/v<version>-<n>` tag. A fix made later on a patched branch is a new commit on it, tagged with the next number.

## Syncing

When crates.io has a newer release, a scheduled workflow opens an issue here. On `main`, one command brings it in:

```sh
scripts/sync.sh 0.3.14
```

It downloads the release and checks it against the checksum crates.io gives. It commits it on `upstream`, tagged `upstream/v0.3.14`, and makes `patched/v0.3.14` from that tag with the newest patched branch's commits cherry-picked onto it. Then it checks the patched files' formatting, runs `kinetic.rs`'s tests, and tags the result `patched/v0.3.14-1`. It works in temporary worktrees, pushes nothing, and ends by printing the push, atomic so that the four refs arrive together, and the commit for Ritornel to pin. Run again after a failure, it goes on from what the last run finished. A conflict stops it, leaving the worktree to finish the cherry-pick in. After a manual finish, check and tag as the script's last step does:

```sh
git diff --name-only -z upstream/v0.3.14 HEAD -- '*.rs' | xargs -0 rustfmt --edition 2024 --check
rustc --edition 2024 --test -D warnings -o /tmp/kinetic src/linux/wayland/kinetic.rs && /tmp/kinetic
git tag -a -m "gpui-pre-linux 0.3.14 with Ritornel's patch" patched/v0.3.14-1
```

## Ritornel's pins

Ritornel pins `gpui-pre`, `gpui-pre-platform` and every other `gpui-pre-*` crate to one version, and the patched branch it takes `gpui-pre-linux` from must be of that same version. A bump in Ritornel moves them all together: every `gpui-pre-*` pin to `=0.3.14`, and the `[patch.crates-io]` rev in `apps/desktop/Cargo.toml` to the commit of `patched/v0.3.14-1`. A `[patch]` whose version no longer matches is ignored with only a warning, so Ritornel's `scripts/desktop.sh check` fails unless Cargo resolves `gpui-pre-linux` from this repository.

## CI

`kinetic.rs` depends on the standard library alone, so its tests run with `rustc --test` and need no GPUI build. The crate's own tests need GPUI's test support, which the published crate leaves out. A workflow on a patched branch would be a change to the crate, so `patched.yml`, here on `main`, checks out every `patched/*` branch and checks it. It runs when `main` changes, every day, and on demand with `gh workflow run patched.yml`. `upstream.yml` looks for a newer release every day.
