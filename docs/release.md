# Release

## Versioning

- Git tag: `v2.0.0`.
- Cargo workspace version: `2.0.0` (must match the tag without `v`).
- Image tags: `2`, `2.0`, `2.0.0`, `latest` for the dev image; the same with
  `-slim` for the slim image.
- `latest` follows the default branch and points at the dev variant.

## Workflows

| Workflow | Trigger | Output |
| --- | --- | --- |
| `ci.yml` | push / PR | fmt, clippy, tests, cargo-deny/audit, shellcheck, hadolint, actionlint, Compose config, slim image smoke |
| `release.yml` | tag `v*` | Linux/macOS/Windows `dshctl`, SHA256SUMS, cosign signature, build provenance, GitHub Release |
| `publish-image.yml` | main, tags, weekly | multi-arch dev + slim GHCR images, provenance, image SBOM, cosign signature |

## Release checklist

1. Update `CHANGELOG.md` and the version in `Cargo.toml`.
2. Run `cargo fmt --all -- --check`, `cargo clippy ... -D warnings`,
   `cargo test --workspace --locked`.
3. Build both image targets locally (or rely on CI).
4. Commit, push, and create an annotated tag `vX.Y.Z`.
5. Confirm `release.yml` created the release and uploaded all assets.
6. Confirm `publish-image.yml` pushed `X`, `X.Y`, `X.Y.Z`, `latest` and the
   `-slim` variants.
7. Set the GHCR package visibility to Public (one-time, manual).
8. Verify anonymous pull and the cosign signatures.
9. Update the README release/link references if a new minor line is introduced.

## Signatures

Images and release checksums are signed with cosign keyless (GitHub OIDC). Example
verification:

```bash
cosign verify ghcr.io/rin721/dsh-docker:2 \
  --certificate-identity-regexp '^https://github.com/rin721/dsh-docker/.github/workflows/publish-image.yml@refs/(heads/main|tags/v.*)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

cosign verify-blob SHA256SUMS \
  --certificate SHA256SUMS.pem \
  --signature SHA256SUMS.sig \
  --certificate-identity-regexp '^https://github.com/rin721/dsh-docker/.github/workflows/release.yml@refs/tags/v.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

If cosign is unavailable, verify `SHA256SUMS` manually. Third-party mirrors are only
a transport layer.

## Docker Hub / other registries

GHCR is the primary registry. To mirror to Docker Hub or a private registry, pull the
signed GHCR digest and push it with your own credentials; the project does not require
extra registry secrets in CI.

## Rollback

- Keep the previous image digest and tag.
- `release.yml`/GHCR do not delete old tags automatically.
- `dshctl update` does not prune images.
- For a source rollback, check out the previous tag and use its Compose file; the
  `.runtime` data is forward-compatible.
