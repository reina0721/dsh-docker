# Architecture

## Components

| Component | Location | Responsibility |
| --- | --- | --- |
| `dshctl` | Host | Init, deploy, image delivery, auth, diagnostics, backup/restore, migration, self-update |
| `dsh-entrypoint` | Container | PID 1 helper, persistent home setup, DSH supervision, Rust reverse proxy |
| Rust gateway | Inside `dsh-entrypoint` | Basic Auth, Host/Origin validation, HTML compat injection, WebSocket/SSE, upstream proxy |
| DSH | Inside image | `dsh web --host 127.0.0.1 --port 3079` |
| Docker Compose | Host | One `dsh` service, bind mounts, port mapping, healthcheck |

## Topology

```text
Browser
  │ HTTP/HTTPS
  ▼
external reverse proxy (optional, TLS)
  │ HTTP
  ▼
127.0.0.1:3080
  dsh container
    tini (PID 1)
      dsh-entrypoint
        ├── Rust gateway 0.0.0.0:3080
        └── dsh web 127.0.0.1:3079
```

Only the gateway is reachable from outside the container. DSH remains loopback-only,
and the Compose port mapping only publishes the gateway port.

## Request lifecycle

1. The gateway reads the request line and Host header; absolute-form URIs and
   `CONNECT` are rejected.
2. Basic Auth is checked. Missing users produce 503 (fail closed); bad or missing
   credentials produce 401 with `WWW-Authenticate`.
3. Host/Origin trust is checked with the same contract as the v1 Node proxy:
   - Host must be a bare authority.
   - `Sec-Fetch-Site: cross-site` is rejected.
   - If Origin is present, its authority must equal Host's authority.
4. `/__dsh_http_compat.js` serves the embedded browser shim (or 404 when disabled).
5. Normal requests are forwarded to `127.0.0.1:3079` with Host/Origin internalized,
   `Authorization` removed, and `X-Forwarded-*` set.
6. HTML responses are buffered up to 8 MiB and the shim is injected. Non-HTML,
   compressed, non-UTF-8, HEAD, 206, and oversized responses are passed through or
   rejected explicitly.
7. WebSocket/Upgrade requests are forwarded; after a 101 response the two upgraded
   streams are copied bidirectionally.
8. Upstream errors produce 502, timeouts 504, and missing auth users 503.
9. DSH's web token is appended to upstream requests until its auth cookie is present;
   the token is redacted from supervision logs.

## Process supervision

`tini` remains PID 1 and reaps zombies. `dsh-entrypoint`:

- prepares the persistent home (symlinks, safe migration, GnuPG socket cleanup);
- starts `dsh web` in its own process group and waits up to 90 seconds for readiness;
- captures the per-process `?token=` printed by `dsh web`, redacts it from logs, then
  starts the gateway with that token;
- the gateway appends the token until DSH returns its `dsh-auth-*` cookie, then
  reuses that captured cookie and replaces stale client cookies; a Basic-Auth browser
  can open `/` directly without the tokenized URL and recovers after a restart;
- forwards SIGTERM/SIGINT to the DSH process group, waits 15 seconds, then kills;
- stops the gateway and exits if either side fails, letting Compose restart the
  container.

## Persistence

The repository `.runtime/` is mounted as four directories:

| Host | Container |
| --- | --- |
| `.runtime/workspace` | `/workspace` |
| `.runtime/dsh-home` | `/home/node/.dsh` |
| `.runtime/home` | `/home/node/.persist` |
| `.runtime/auth` | `/home/node/.dsh-auth` (read-only) |

`dsh-entrypoint` links `~/.ssh`, `~/.config`, `~/.gitconfig`, etc. into
`/home/node/.persist`. It never copies or deletes a non-empty source before a
successful copy, unlike the old `cp -a -n ... && rm -rf` sequence.

The auth directory is mounted rather than the auth file so an atomic rename on the
host is visible to the gateway.

## Design decisions

- **Rust instead of Caddy + Node proxy**: one process, one dependency surface, one
  place for auth/trust/injection. Caddy's TLS features are intentionally delegated
  to the external reverse proxy, matching the v1 contract.
- **Hyper directly instead of a framework**: precise control of hop-by-hop headers,
  streaming, and upgrades with a small dependency set.
- **CLI calls Docker Compose**: Compose remains the source of truth and keeps
  BuildKit/progress/auth behavior; the CLI adds validation, safety, and UX.
- **`.env` remains**: existing users can inspect and edit Compose configuration.
- **bcrypt htpasswd remains**: existing Caddy hashes work after a file rename.
- **Single container**: simpler operations and a smaller attack surface. Updating
  the gateway requires pulling a new image, which CI publishes automatically.
