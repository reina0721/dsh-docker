# Security Policy

## Supported versions

| Version | Supported |
| --- | --- |
| 2.x | Yes |
| 1.x | No (migration only) |

## Reporting a vulnerability

Do not open a public issue. Use GitHub's private security advisory flow for
`reina0721/dsh-docker`, or contact the maintainer through the repository profile.

Please include:

- affected version and image digest;
- deployment topology (TLS proxy, bind address, auth configuration);
- reproduction steps and impact;
- any suggested fix.

## Security model

- The Rust gateway is plain HTTP. Terminate TLS in an external reverse proxy and
  keep the default loopback binding.
- Basic Auth uses bcrypt hashes stored in `.runtime/auth/users.htpasswd` (0644).
  The gateway caches successful credentials in memory and fails closed if no users
  are configured.
- SSH keys live in `.runtime/home/ssh` (0700, private keys 0600). `dshctl ssh` never
  prints private material; `dshctl ssh list` shows public keys/fingerprints only.
- Backups contain SSH keys, cloud/Git credentials, DSH credentials, and auth hashes.
  The archive is mode 0600 but must be protected/encrypted as a secret.
- Docker socket access is equivalent to host root. Restrict it to trusted users.
- Images and releases are built by GitHub Actions, checked with `cargo deny`/audit,
  scanned by CI, and signed with cosign keyless. Third-party mirrors are transport
  only; verify checksums/signatures.

## Hardening checklist

- TLS in front; `BIND_ADDRESS=127.0.0.1`.
- Strong unique passwords; keep `AUTH_BCRYPT_COST` at 12–14.
- Restrict/rotate SSH and cloud credentials in `.runtime/home`.
- Back up and encrypt `.runtime`.
- Keep the image and `dshctl` updated.
- Monitor `dshctl logs` for repeated 401/403 responses.
