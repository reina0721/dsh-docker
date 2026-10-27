## Summary

<!-- What changed and why? -->

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `docker compose config`
- [ ] Docker image/smoke test (if applicable)

## Checklist

- [ ] Tests added/updated for behavior changes
- [ ] Docs/CHANGELOG updated
- [ ] No secrets, `.env`, `.runtime`, or backups committed
- [ ] Commits signed off (`git commit -s`)
