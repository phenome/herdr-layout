# Herdr Layout

Small Herdr plugin for three saved tab layouts.

## Install

```sh
herdr plugin install phenome/herdr-layout
```

Users do not need Bun. The plugin installs the `herdr-layout` release binary for version `0.0.1` from GitHub Releases.

0.0.1 binaries are unsigned raw release assets. Your OS may warn before first run; only install releases you trust.

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

- `firstUsesCurrentTab`: `true` to reuse the current idle tab for the first target.
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

Maintainers need Bun:

```sh
bun install
bun test
bun run typecheck
bun run build:release          # current host only
bun run build:release:all      # all release assets
bun run build:release -- linux-x64
```

Release from CI by pushing a version tag:

```sh
git tag v0.0.1
git push origin v0.0.1
```
