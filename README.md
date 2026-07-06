# Herdr Layout

Small Herdr plugin for three saved tab layouts.

## Install

```sh
herdr plugin install phenome/herdr-layout
```

Users do not need Bun, Node, or Rust. The plugin installs the `herdr-layout` release binary for the version declared in `herdr-plugin.toml`.

Binaries are unsigned raw release assets. Your OS may warn before first run; only install releases you trust.

## Configure one layout

Find the plugin config directory:

```sh
herdr plugin config-dir herdr-layout
```

Create `config.yaml` in that directory:

```yaml
layouts:
  "1":
    firstUsesCurrentTab: true
    tabs:
      - label: api
        command: bun run dev
      - label: tests
        command: bun test --watch
```

`layouts` is a map keyed by quoted layout slots: `"1"`, `"2"`, `"3"`.

Each layout has:

- `firstUsesCurrentTab`: `true` to reuse the current tab for the first target when it is idle or already running that target.
- `tabs`: non-empty list of tab targets.
- `label`: Herdr tab label to reuse or create.
- `command`: shell-line string Herdr runs in that tab.

`command` is passed as a shell line, not an argv array. Quote it like you would in your shell. Missing binaries fail in the shell.

## Repo override

Put `.herdr-layout.yaml` or `.herdr-layout.yml` in a repo to override slots for that repo. Herdr Layout uses the nearest ancestor override from the active pane cwd.

```yaml
layouts:
  "1":
    firstUsesCurrentTab: false
    tabs:
      - label: web
        command: npm run dev
      - label: db
        command: docker compose up db
```

Slots present in the repo override replace the global slot. Other slots still come from global `config.yaml`.

## Keybindings

Add bindings to Herdr `config.toml`:

```toml
[[keys.command]]
key = "prefix+1"
type = "plugin_action"
command = "herdr-layout.apply-1"
description = "Apply layout 1"

[[keys.command]]
key = "prefix+2"
type = "plugin_action"
command = "herdr-layout.apply-2"
description = "Apply layout 2"

[[keys.command]]
key = "prefix+3"
type = "plugin_action"
command = "herdr-layout.apply-3"
description = "Apply layout 3"
```

## Development

Maintainers need Bun for release tooling and Cargo for the runtime:

```sh
bun install --frozen-lockfile
cargo test --locked
cargo check --all-targets --locked
cargo build --release --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

Release asset renaming happens in GitHub Actions; there is no repo-local build script.

Releases are automatic on `main`. Use Conventional Commits:

- `fix:` / `perf:` → patch
- `feat:` → minor
- `BREAKING CHANGE:` or `!` → major

After tests pass, semantic-release bumps `package.json` and `herdr-plugin.toml`, updates `CHANGELOG.md`, tags `vX.Y.Z`, creates the GitHub Release, and uploads binaries.
