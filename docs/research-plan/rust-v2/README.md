# Rust v2 plan

This directory records the implementation plan for the v2 Rust rewrite.

## Goal

Replace the v1 host shell scripts, Caddy gateway, and Node compatibility proxy with:

- `dshctl`: a static host CLI;
- `dsh-entrypoint`: a Rust container init/supervisor and reverse proxy;
- a single-container Compose topology;
- reproducible multi-platform releases and multi-arch images;
- domestic download/image/build acceleration and mature project documentation.

## Decisions

- Rust gateway on `hyper`/`tokio`; Caddy and the Node proxy are removed.
- DSH stays loopback-only; the gateway is the only exposed process.
- CLI shells out to `docker compose` (Compose remains the source of truth).
- `.env` remains the configuration format; auth remains bcrypt htpasswd.
- v2.0.0 is a breaking release with a migration command and guide.
- `dev` and `slim` image targets; GHCR is primary.
- Chinese-primary README + English mirror; bilingual key docs.

## Acceptance

- [x] Rust workspace compiles, tests pass, Clippy clean.
- [x] Gateway integration tests cover auth, trust, injection, compat asset, and
      fail-closed behavior.
- [x] Compose has one `dsh` service, no Caddy/Node proxy.
- [x] `dshctl init/user/backup/restore/status` smoke-tested.
- [ ] CI image build and release workflows exercised in GitHub Actions.
- [ ] Real Docker deployment on Linux/macOS/Windows hardware.
- [ ] GHCR package visibility and signatures verified after the first release.

See the repository [docs index](../../README.md) for user documentation.
