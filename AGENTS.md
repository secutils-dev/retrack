# Retrack agent guide

Retrack contains a Rust tracking API, shared `retrack-types`, and a Node/Playwright web scraper.
Paths and commands here are relative to the Retrack repository root, including when it is a Secutils submodule.

## Task guides

Read each applicable guide before changes to its surface.

| Trigger                                                                         | Read                                                                               |
|---------------------------------------------------------------------------------|------------------------------------------------------------------------------------|
| Rust/NPM dependencies, Node versions, or Docker base-image upgrades             | [dev/agent-guides/dependency-upgrades.md](dev/agent-guides/dependency-upgrades.md) |
| Camoufox versions, its image/launcher, or Firefox installation/runtime failures | [dev/agent-guides/camoufox.md](dev/agent-guides/camoufox.md)                       |
| JS runtime benchmarks, scenario changes, or performance analysis/history        | [dev/agent-guides/performance.md](dev/agent-guides/performance.md)                 |

For product-wide upgrades inside Secutils, read the parent `AGENTS.md` and its dependency guide for stage/pointer coordination.
Keep local implementation guidance here and product-wide coordination in Secutils.

## Commands and verification

Use `make help` for targets and flags. Read `package.json` for workspace scripts.
Run NPM installation from this root so the scraper uses the shared workspace lockfile.
`make dev-up` starts PostgreSQL only. The full application stack belongs to Secutils.
`make dev-down` and `make db-reset` delete development data.

Use [.husky/pre-push](.husky/pre-push) for the local push gate.
Use [.github/workflows/ci.yml](.github/workflows/ci.yml) for CI checks and path exclusions.
Rust formatting requires nightly, as expressed by `make fmt`:

```bash
cargo +nightly fmt --all -- --check
```

After SQLx macro query changes, run `make dev-up`, `make db-prepare`, and `make db-prepare-check`.
Include generated `.sqlx/` changes with the query changes.
Runtime SQL queries do not use that cache.

Completion requires passing checks for each changed surface and any additional gates in its guide.
Review generated artifacts and the final diff.
If a required check is blocked, report the command and blocker as incomplete verification.

When a commit is requested, use conventional commits: `chore(deps)` for dependencies and `chore(docker)` for image pins.
Inspect the local hooks for their actual wiring.
