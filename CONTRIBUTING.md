# Contributing

Thanks for helping improve DSH Docker.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
```

Docker is required for image/deployment tests. The gateway integration test runs
without Docker.

## Pull requests

- Keep changes focused and explain the user-visible behavior.
- Add tests for gateway/auth/config/backup changes.
- Do not capture Docker pull/build output in a variable; live progress is a
  regression-tested requirement.
- Update docs/CHANGELOG for user-visible changes.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `chore:`).
- Sign off commits with the Developer Certificate of Origin (`git commit -s`).

## Reporting bugs

Include:

- `dshctl version`
- OS/architecture and Docker/Compose versions
- `dshctl doctor` output
- relevant `dshctl logs --tail 200` lines (redact secrets)

Security issues: see [SECURITY.md](SECURITY.md), not a public issue.

## Code style

- Rust 2024 edition, `rustfmt`, Clippy warnings denied.
- No `unsafe`; use safe Unix wrappers (`nix`).
- Keep the public CLI and `.env` compatible unless a breaking release is planned.
- Do not commit `.env`, `.runtime`, credentials, backups, or generated archives.
