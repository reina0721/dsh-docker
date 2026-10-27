# Troubleshooting

Start with:

```bash
dshctl doctor
dshctl doctor --network
dshctl status --json
dshctl logs --tail 200
```

## Port already in use

`deploy` binds the configured host port before starting. If another process owns it,
change `DSH_PORT` in `.env`. If the port belongs to this project's old container,
the CLI allows recreation and `--remove-orphans` removes the old Caddy service.

```bash
ss -ltnp 'sport = :3080'
```

## 401 on every request

Expected without credentials. Verify the user and file:

```bash
dshctl user list
cat .runtime/auth/users.htpasswd
```

If `dshctl user add` succeeded but the browser still fails, check that the auth
directory is mounted (not a single file) and that the file is readable by UID 1000.

## 401 "dsh web authentication required"

That message comes from DSH, not the gateway: Basic Auth succeeded but DSH's web
token/cookie was not accepted. Check that startup logs contain
`dsh web: ...?token=<redacted>` and that the gateway started after it. Clear cookies
for the site and reopen the gateway URL. A browser must be allowed to store the
`dsh-auth-*` cookie; the gateway only injects the token until that cookie exists.

## 503 "no authentication users configured"

The gateway fails closed. Create a user:

```bash
dshctl user add alice
```

No restart is required; the file is reloaded by inode/mtime.

## 403 with `x-dsh-proxy-reject`

| Reason | Cause |
| --- | --- |
| `invalid-host` | Host missing or not a bare authority |
| `cross-site` | `Sec-Fetch-Site: cross-site` |
| `origin-host-mismatch` | Origin authority differs from Host |
| `invalid-origin` | Origin is malformed or `null` |

Configure the reverse proxy to preserve the original Host and Origin. Do not rewrite
them to `localhost` at the edge.

## WebSocket or SSE disconnect

- The reverse proxy must forward `Upgrade`/`Connection`.
- Disable response buffering for SSE (`proxy_buffering off` in Nginx).
- Increase read timeouts for long-lived agent sessions.
- Check `dshctl logs` for `upstream upgrade failed` or connection-closed messages.

## GHCR pull hangs

```bash
dshctl image check
dshctl deploy --mirror cn
```

A fast manifest but stalled `Downloading/Waiting` usually means the layer CDN
(`pkg-containers.githubusercontent.com`) or outbound bandwidth. Configure
`DSH_IMAGE_MIRRORS` or a registry proxy. Docker daemon `registry-mirrors` only
applies to Docker Hub, not GHCR.

## Image is rejected as incompatible

v1 images do not carry the v2 runtime label. Pull/rebuild the v2 image:

```bash
dshctl migrate
dshctl deploy
# or
dshctl rebuild
```

## Permission denied on `.runtime`

The container runs as UID/GID 1000. If the host user differs:

```bash
sudo chown -R 1000:1000 .runtime
# or set RUNTIME_UID/RUNTIME_GID and run dshctl as root once
```

## SELinux

If bind mounts are denied, inspect `ausearch`/`journalctl` and apply a container
label, for example:

```bash
chcon -Rt container_file_t .runtime
```

Do not disable SELinux globally.

## Windows and macOS

- Windows: run from WSL2 for the best file permission/symlink behavior. Docker
  Desktop must be running with the project drive shared. Backup symlinks are
  best-effort on NTFS.
- macOS: bind mounts through Docker Desktop are slower than a Linux volume; keep the
  workspace small or use a named volume.

## Docker daemon unreachable

```bash
docker info
```

Ensure the daemon is running and the current user is in the `docker` group (or
configure rootless Docker). `dshctl doctor` reports this explicitly.

## Logs

The container uses the `json-file` driver with 10 MiB × 3 rotation. `dshctl logs`
follows the Compose service. Gateway logs include method, path, status, peer, and
auth result; they never include the Authorization header or passwords.
