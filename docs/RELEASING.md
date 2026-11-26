# Releasing Triceratops

A release is a `v*` tag on `master`: the Build workflow makes the Linux AppImage and the
Windows zip, runs the MCP smoke test on both, publishes the GitHub release with the notes
from `CHANGELOG.md`, and restamps the website with the new version and download links.
macOS builds stopped after 0.3.0 and are not made.

## Before tagging

1. **Version.** Bump `version` in the root `Cargo.toml`, build once so `Cargo.lock`
   follows, and commit both. The tag (`vX.Y.Z`) and this version must match.
2. **Changelog.** In `CHANGELOG.md`, rename `## Unreleased` to `## X.Y.Z - YYYY-MM-DD` and
   read it as a user would: this section becomes the release notes *and* the text the
   update dialog shows everyone on an older version. The release job stops if the
   section is missing (`scripts/changelog_section.py X.Y.Z` prints what it will publish).
3. **Checks.** `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, and CI green on the pull request (Linux and Windows).

## The update path (try it for real at the next release)

0.4.0 is the first version that updates itself; its AppImage swap and restart are covered
by unit tests only, and the Windows download has not run on a real Windows machine. With
the next release, check both before announcing it:

- **AppImage.** Keep a copy of the 0.4.0 AppImage. After the new release is published,
  start the 0.4.0 copy: the update dialog should open by itself (or use
  Help › Check for Updates…) with the new version's notes. Choose *Update and restart*:
  the download completes, the file is replaced in place (still executable), *Restart now*
  asks about unsaved changes and starts the new version, and Help › About shows it.
  Also try it from a folder you cannot write to: it should fall back to Downloads.
- **Windows.** On Windows, *Download* puts the verified zip in Downloads and opens
  Explorer on it.
- **Before tagging, without publishing:** serve a fake release list locally and point a
  build at it. Put the JSON at `repos/libano23/triceratops-rs/releases` under a folder,
  with one release newer than the build, an asset named `…-x86_64.AppImage` whose `size`
  and `digest` (`sha256:<hex>` of the file) match a file served from the same folder,
  then run `python3 -m http.server 8765` there and start Triceratops (as an AppImage, so
  `APPIMAGE` is set) with `TRICERATOPS_UPDATE_URL=http://127.0.0.1:8765`.

## Tagging and after

```sh
git tag -a vX.Y.Z -m "Triceratops X.Y.Z" && git push origin vX.Y.Z
```

Then check that the release has the two files and the changelog section as its notes,
that the website shows the new version, and add a fresh `## Unreleased` section at the
top of `CHANGELOG.md` for the next changes.

The social preview image (`site/assets/social-preview.png`, new logo since 0.3.0) can
only be uploaded by hand in the repository's Settings › General › Social preview.
