# Use Rust release binaries

Herdr Layout was first published as Bun-compiled TypeScript binaries, but now ships Rust release binaries so plugin users do not need Bun, Node, or a language toolchain installed and the downloaded artifacts stay smaller. The Herdr manifest version maps to a GitHub release tag in `phenome/herdr-layout`, and install-time OS helpers download the matching platform/architecture asset before Herdr registers the runtime actions.

## Considered Options

- Require Bun at runtime: fastest from the current local plugin, but every user must install Bun.
- Build during plugin install: keeps the repo smaller, but shifts toolchain setup onto users.
- OS scripts as runtime: avoids external runtimes, but creates multiple implementations and weak YAML handling.
- Rust release binaries: keeps one runtime implementation, avoids user toolchains, and produces smaller release artifacts.

## Consequences

Release publishing must build and attach the supported target binaries: Windows x64, macOS x64/arm64, Linux x64/arm64, and Linux musl x64. Local development uses Cargo for the runtime; Bun remains only for repo-local release tooling.
