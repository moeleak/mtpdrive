# MTPDrive Repository Instructions

These instructions apply to the entire workspace.

## Architecture

- Keep the existing `mtpdrive-core`, `mtpdrive-app`, and `mtpdrive-cli` crate
  structure unless the user explicitly requests another crate.
- Keep both binary `main.rs` files as thin entry points. They may initialize the
  runtime, parse arguments, and delegate to a library entry point, but must not
  accumulate UI, command, IPC, or domain logic.
- In `mtpdrive-app`, separate the application state machine, page views,
  updater, tray integration, single-instance handling, and daemon adapter into
  focused modules.
- In `mtpdrive-cli`, keep argument definitions, command dispatch, and output
  rendering separate.
- Put shared IPC behavior, typed daemon-client operations, domain formatting,
  and daemon-readiness logic in `mtpdrive-core`.
- Do not make `mtpdrive-core` depend on Iced, Material UI, tray integration, or
  the macOS `.app` bundle layout.

## Compatibility

- Structural refactors must preserve GUI behavior, CLI arguments and output,
  IPC JSON, settings formats, binary names, and `.app` helper paths unless the
  user explicitly requests a behavior or protocol change.
- Do not remove existing public APIs when adding typed convenience APIs. In
  particular, retain the low-level `DaemonClient::request` interface.
- Tray changes must preserve the stable tray ID, filtering to the active tray
  icon, in-place menu updates, and exact validation of the single-instance
  `show` message.

## Tests

- Do not place test implementations directly in `src/*.rs` files.
- Put public-behavior integration tests in each crate's `tests/*.rs` files.
- Put tests that require access to private implementation details in
  `tests/unit/*.rs`. Include them from the relevant source module with
  `#[cfg(test)]` and `#[path = "../tests/unit/..."]`.
- Do not expand production public APIs solely to make tests accessible.
- Keep crate-owned tests inside that crate. The vendored `vendor/fractal-nfs`
  crate is outside this test-layout migration rule.

## Validation

- For Rust changes, run:

  ```sh
  cargo fmt --all -- --check
  cargo test --workspace
  ```

- Run targeted Clippy checks for the changed app or CLI package through the
  project toolchain, for example:

  ```sh
  nix develop -c cargo clippy -p mtpdrive-app --all-targets --no-deps -- -D warnings
  nix develop -c cargo clippy -p mtpdrive-cli --all-targets --no-deps -- -D warnings
  ```

- When packaging code, binary entry points, or helper paths change, run
  `nix develop -c cargo xtask app` and verify the Universal architectures and
  code signature.
- When refactoring the CLI, compare `mtpdrive --help` and representative command
  output before and after the change.

## Worktree Safety

- Inspect the worktree before editing and preserve unrelated or uncommitted user
  changes.
- Treat the current tray focus-stealing fix as required baseline behavior; do
  not regress or discard it during later refactors.
