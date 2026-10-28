# Changelog

All notable changes are documented here. The format follows Keep a Changelog and
the project uses Semantic Versioning.

## [2.0.0] - 2026-10-27

### Breaking

- Replaced the Caddy gateway and Node compatibility proxy with a Rust gateway inside
  the `dsh` container. The `gateway` Compose service and `Caddyfile.gateway` are gone.
- Replaced all host shell scripts with the `dshctl` binary. `./scripts/*.sh` no longer
  exists; see `docs/migration-v1-to-v2.md` for the command mapping.
- The default image is now `ghcr.io/reina0721/dsh-docker:2` and must carry the
  `io.github.reina0721.dsh-docker.runtime=2` label. v1 caches are rejected.
- The canonical auth file is `.runtime/auth/users.htpasswd`; `users.caddy` remains a
  read fallback and is migrated by `init`/`migrate`.
- New passwords default to a minimum of 12 characters.
- `dshctl stop` keeps the container; use `dshctl down` for the old `stop.sh` behavior.

### Added

- Rust `dshctl` with init, deploy, lifecycle, status, doctor, logs, shell, exec,
  update, self-update, rebuild, user, backup, restore, image, config, migrate,
  completion.
- Persistent SSH/Git identity management: `dshctl ssh list/generate/import/remove/test`
  and `dshctl git setup/show`, with private-key-safe output and 0600/0644 modes.
- The supervisor captures DSH's per-process web token, redacts it from logs, and the
  gateway injects it until DSH sets its auth cookie, so the Basic-Auth URL works
  directly without reopening the tokenized URL.
- Node/DSH tooling is installed under `/opt/dsh`; mounting the home directory no
  longer hides `dsh`, `pnpm`, `tsc`, or `tsx`.
- Rust container entrypoint: safe persistent-home migration, DSH supervision, signal
  forwarding, healthcheck, and a Hyper reverse proxy.
- Basic Auth with bcrypt verification cache, single-flight verification, and
  inode/mtime hot reload without restarting the container.
- Host/Origin validation, HTML compatibility injection, WebSocket upgrades, SSE
  streaming, hop-by-hop header filtering, and explicit 502/503/504 behavior.
- Backup/restore with staged extraction, traversal protection, and rollback.
- `dev` and `slim` Docker targets; multi-arch GHCR publishing, SBOM, provenance, and
  cosign signatures.
- GitHub Releases for Linux/macOS/Windows `dshctl` with SHA256SUMS and cosign
  signatures, plus `install.sh` and `install.ps1`.
- China mirror support for downloads, images, apt, npm, pip, Go, Rust, and Cargo.
- Bilingual README, architecture/configuration/CLI/security/troubleshooting docs,
  reverse-proxy examples, and community files.

### Fixed

- Docker pull/build output is inherited, never captured, so progress remains live.
- Persistent-home migration no longer deletes a non-empty source before a successful
  copy.
- The auth file directory is mounted, so atomic rewrites are visible to the gateway.
- Windows line endings are normalized with `.gitattributes`; executable-bit problems
  are removed with the Rust binary.

### Removed

- Caddy, the Node proxy, `start-dsh-web.sh`, and all `scripts/*.sh`.
- `GATEWAY_CADDY_VERSION` (ignored if still present).
