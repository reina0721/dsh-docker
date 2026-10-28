# Migrating from v1 to v2

v2 removes the shell scripts, the Caddy gateway, and the Node proxy. It is a breaking
change. The data under `.runtime/` is preserved.

## Before you start

```bash
# From the v1 checkout
./scripts/backup.sh
```

Copy the backup off the host or encrypt it. It contains SSH keys, Git/CLI
credentials, DSH credentials, and the Basic Auth hashes.

## Command mapping

| v1 | v2 |
| --- | --- |
| `./scripts/deploy.sh` | `dshctl deploy` |
| `./scripts/update.sh` | `dshctl update --source --backup` (source) or `dshctl update` |
| `./scripts/rebuild.sh` | `dshctl rebuild` |
| `./scripts/check.sh` | `dshctl status` |
| `./scripts/doctor.sh` | `dshctl doctor` |
| `./scripts/check-image.sh` | `dshctl image check` |
| `./scripts/create-user.sh` | `dshctl user add NAME` |
| `./scripts/list-users.sh` | `dshctl user list` |
| `./scripts/remove-user.sh NAME` | `dshctl user remove NAME` |
| `./scripts/backup.sh` | `dshctl backup` |
| `./scripts/stop.sh` | `dshctl down` |
| `bash scripts/fix-home-permissions.sh` | `dshctl doctor --fix` |
| `bash scripts/migrate-home-state.sh` | `dshctl migrate` / automatic on startup |
| `./scripts/self-test.sh` | `cargo test --workspace` |
| `./scripts/prepare-release.sh` | CI workflows |

## Steps

1. **Stop the old stack**

   ```bash
   ./scripts/stop.sh
   ```

   If you no longer have the v1 checkout, `docker compose down --remove-orphans` in
   the old project directory is equivalent.

2. **Get v2**

   ```bash
   git fetch --tags
   git checkout v2.0.0
   # or use the install script into a fresh directory and copy .runtime/.env
   ```

3. **Inspect the migration**

   ```bash
   dshctl migrate --dry-run
   ```

4. **Apply it**

   ```bash
   dshctl migrate
   ```

   This:

   - copies `auth/users.caddy` to `auth/users.htpasswd` (legacy file kept);
   - updates `DSH_IMAGE=...:latest` to `...:2` with a `.env` backup;
   - creates missing runtime directories and permissions.

5. **Deploy**

   ```bash
   dshctl deploy
   ```

   `up --remove-orphans` removes the old Caddy container. The DSH image must be v2;
   the CLI rejects a cached v1 image without the runtime label.

6. **Verify**

   ```bash
   dshctl status
   dshctl doctor
   ```

   You should see one `dsh` service, HTTP 401 without credentials, and your existing
   users still working.

## Environment changes

Removed/ignored:

- `GATEWAY_CADDY_VERSION` — no Caddy.
- Caddy-specific paths are gone; the auth file is `users.htpasswd`.

New/changed:

- `DSH_IMAGE` should be `ghcr.io/reina0721/dsh-docker:2`.
- `DSH_IMAGE_VARIANT=dev|slim`.
- `DSH_IMAGE_MIRROR` and `DSH_IMAGE_MIRRORS` for China.
- `DSH_AUTH_FILE` defaults to `/home/node/.dsh-auth/users.htpasswd` inside the
  container; Compose owns this.
- `AUTH_MIN_PASSWORD_LENGTH` defaults to 12 for new passwords.
- `RUST_VERSION` replaces `RUST_TOOLCHAIN` for the build.

Existing `RUNTIME_DIR`, `RUNTIME_UID/GID`, `BIND_ADDRESS`, `DSH_PORT`, proxy
variables, and DSH settings continue to work.

## What is not migrated

- The Caddy process/container.
- Old shell wrappers.
- Any custom edits to `scripts/`, `dsh-web-proxy.mjs`, `start-dsh-web.sh`, or
  `Caddyfile.gateway`. Port them to the Rust gateway via a PR, or keep a v1
  deployment while you plan the move.

## Rollback

1. Stop v2: `dshctl down`.
2. Check out the v1 tag and restore its `compose.yaml`/scripts.
3. Use a v1 image tag/digest.
4. `.runtime` is not deleted by v2. If you migrated the auth file, the legacy
   `users.caddy` is still present and v1 can read it.
5. Restore the v1 backup only if you need to roll back data as well.

## Windows notes

The v1 checkout often lost executable bits on Windows. v2 avoids shell scripts
entirely; install `dshctl` and run it from PowerShell. For file permissions and
symlinks, WSL2 is still the most reliable environment.
