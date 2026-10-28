# Security

## Trust boundary

```text
Internet -> TLS reverse proxy -> loopback gateway -> loopback DSH
```

The gateway is a plain-HTTP Basic Auth proxy. It does not implement TLS, ACME, or
certificate management. Do not expose `DSH_PORT` directly to the public internet
over plain HTTP.

Recommended:

- keep `BIND_ADDRESS=127.0.0.1`;
- terminate TLS in Nginx/Caddy/Traefik/1Panel;
- preserve `Host` and `Origin`;
- apply rate limiting at the TLS proxy.

## Basic Auth

- bcrypt hashes in `.runtime/auth/users.htpasswd` (0644).
- Plaintext passwords are never written to disk or passed in process arguments.
- `--password-stdin` and `--password-file` exist for automation; prefer a secret
  manager or a 0600 file.
- Successful credentials are cached in memory (TTL 10 minutes, bounded) because
  bcrypt cost 14 is intentionally expensive. Changing the file clears the cache.
- Missing/empty auth database fails closed with HTTP 503.
- Unknown usernames and wrong passwords return the same 401 response.
- Basic Auth sends credentials on every request; HTTPS is mandatory in production.

## Host/Origin and CSRF

The gateway preserves the v1 contract:

- Host must be a bare authority (no userinfo/path/query).
- `Sec-Fetch-Site: cross-site` is rejected.
- Origin, when present, must match Host's authority.
- `Authorization` is stripped before forwarding to DSH.
- Hop-by-hop headers are stripped; WebSocket upgrades are handled explicitly.
- DSH's per-process web token is captured by the supervisor, redacted from logs, and
  injected until DSH returns its `dsh-auth-*` cookie. The gateway then reuses that
  cookie and replaces stale client cookies; an expired cached cookie is dropped on
  upstream 401.

## Credentials in backups

`dshctl backup` archives `.env` and `.runtime/`, which include:

- SSH private keys;
- Git credentials and `.netrc`/`.pypirc`;
- DSH credentials and sessions;
- Basic Auth hashes.

The archive is mode 0600, but it is still a secret. Encrypt it before moving it off
the host. Restore validates paths and refuses traversal.

## Containers and host access

The CLI invokes the Docker CLI; it does not ask for more privileges. Access to the
Docker socket is equivalent to root on the host — restrict it to trusted users.

The container runs as the image's `node` user (UID 1000). Bind mounts must be
writable by that UID. Rootful Docker plus `chown` by `dshctl` is supported; rootless
Docker works if UID mapping is configured.

## Supply chain

- GHCR images are built by GitHub Actions from tagged source.
- Images carry `io.github.reina0721.dsh-docker.runtime=2`; the CLI refuses v1 caches.
- Releases publish SHA256 checksums and cosign signatures.
- Third-party GitHub/GHCR mirrors are transport acceleration only. Verify checksums
  and, when available, cosign signatures. Do not trust an unverified binary.
- Pin actions and dependencies; Dependabot and `cargo deny` run in CI.

## Reporting

Do not open a public issue for a vulnerability. Use GitHub's private security
advisory flow for `reina0721/dsh-docker`, or contact the maintainer through the
repository profile. Include the version, reproduction steps, and impact.
