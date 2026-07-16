# MTPDrive

MTPDrive exposes modern Android MTP devices to macOS as a local NFSv3
volume. It does not require FUSE, a kernel extension, or `libmtp`.

The daemon mounts one volume at `~/MTPDrive`. Finder sees the server and
volume as `MTPDrive` (not `127.0.0.1`). Connected phones appear as top-level
directories and their storage areas appear below them.

The menu-bar app uses `material-ui-rs`: the first page shows device and
storage information, and the second page uses the library's built-in Log
Viewer for structured service logs. Closing the window hides it; clicking the
menu-bar icon shows the same single window again.

## Current platform target

- macOS 13 or newer
- Apple Silicon and Intel
- Android devices exposing standard MTP (Android 5.0 or newer is the intended
  compatibility range)

Unlock the phone and select **File transfer / MTP** after connecting USB.
If macOS Preview, Photos, or Image Capture already owns the phone, close that
app and click **重新扫描**. MTP permits only one desktop process to own a device
session; MTPDrive does not terminate macOS services automatically.

## Development

```sh
nix develop
cargo test --workspace
cargo run -p mtpdrive-cli -- devices
cargo run -p mtpdrive-cli -- daemon --no-mount
cargo run -p mtpdrive-app
```

Build a Universal application and DMG on macOS:

```sh
cargo xtask dmg
```

The ad-hoc signed result is written to
`dist/MTPDrive-<version>-universal.dmg`, alongside its SHA-256 checksum.

## Release

Tags may use `0.1.0` or `v0.1.0`. A matching tag triggers the release workflow,
which builds Apple Silicon and Intel binaries on native GitHub runners, creates
a Universal DMG, generates a changelog from commits since the previous tag,
and publishes the DMG and checksum to GitHub Releases.

## Trademark and artwork notice

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Android is a trademark
of Google LLC. MTPDrive is not affiliated with Google.
