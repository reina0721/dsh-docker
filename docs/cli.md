# CLI reference

```text
dshctl [--dir DIR] [--verbose] <command>
```

`--dir` defaults to the current directory (or the nearest parent containing
`compose.yaml`). `--verbose` prints Docker commands.

## Commands

### `init`

Create `.env`, a missing `compose.yaml`, the `.runtime` tree, and permissions.
Existing files are not overwritten unless `--force` or `--refresh` is used.

### `deploy`

Require Docker/Compose, initialize, ensure at least one auth user, validate Compose,
preflight the host port, prepare the image, start the service, and wait for health.
Flags: `--mode auto|pull|build`, `--mirror global|cn|off`, `--slim`, `--yes`.

Without a TTY and without users, deploy fails with instructions instead of
publishing an unauthenticated gateway.

### Lifecycle

- `up` — validate and start without pulling.
- `down` — Compose `down --remove-orphans`; `.runtime` is untouched.
- `stop` — stop the container, keep it.
- `restart` — restart and wait for health.

### `status [--json]`

Shows Compose services, HTTP status (401 is expected), auth user count, image, and
runtime paths. JSON schema:

```json
{
  "cli_version": "2.0.4",
  "project_dir": "...",
  "runtime_dir": "...",
  "image": "ghcr.io/reina0721/dsh-docker:2",
  "variant": "dev",
  "dsh_version": "0.2.1-rc.5",
  "image_label": "2",
  "services": [],
  "http": { "url": "http://127.0.0.1:3080", "status": 401, "error": null },
  "auth_users": 1,
  "compat": true
}
```

### `doctor [--fix] [--network] [--json]`

Checks Docker/Compose/daemon, `.env`, typed config, Compose config, auth, port,
runtime directory, SSH permissions, image label, and service presence. `--fix`
refreshes the runtime tree and permissions. `--network` checks GHCR and the layer
CDN. Exits non-zero when a check fails.

### `logs` / `shell` / `exec`

`logs [-f] [--tail N]` follows the `dsh` service. `shell` opens bash in the
container. `exec -- cmd args` runs a command in the container.

### `user`

- `user add NAME [--cost N] [--password-stdin|--password-file PATH] [--insecure-password] [--generate]`
  creates or updates a user. Passwords are read without echo by default. `--insecure-password`
  allows setting passwords shorter than the configured `AUTH_MIN_PASSWORD_LENGTH`.
- `user list [--json]`
- `user remove NAME [--yes]`

The gateway reloads the file without a restart. Removing the last user makes the
gateway return 503 (fail closed).

### `ssh`

Manages keys under `.runtime/home/ssh` and never prints private key material.

- `ssh list [--json]` — name, type, private/public, fingerprint, public line.
- `ssh generate [NAME] [--type ed25519|rsa|ecdsa] [--comment TEXT] [--no-passphrase] [--force]`
  — runs `ssh-keygen`; prompts for a passphrase unless `--no-passphrase`.
- `ssh import PRIVATE [--public-key PATH] [--name NAME] [--force]` — copies an
  existing key, validates it, sets 0600/0644, and derives/copies the public key.
- `ssh remove NAME [--yes]`
- `ssh test github|gitlab|gitee|HOST [--key NAME]` — uses
  `StrictHostKeyChecking=accept-new`; GitHub's status 1 greeting is handled.

### `git`

- `git setup [--name NAME] [--email EMAIL] [--key SSH_KEY_NAME]` — writes
  `.runtime/home/git/config`; `--key` sets `core.sshCommand` to the container path.
- `git show` — prints the persistent Git config.

See [ssh-git.md](ssh-git.md) for GitHub setup and multiple-key examples.

### `backup` / `restore`

`backup [OUTPUT] [--quiesce]` writes a 0600 tar.gz containing `.env` and the runtime
directory. `--quiesce` stops the container for a consistent snapshot and restarts it.

`restore ARCHIVE --yes` validates paths, stages the archive, stops the service,
moves the current state aside, restores, fixes permissions, and starts. On failure it
rolls the previous state back.

### `update` / `rebuild`

`update` pulls the latest image and recreates the service. `--source` performs a
clean `git pull --ff-only` first; `--backup` snapshots before updating.

`rebuild` forces a local Docker build (requires `Dockerfile` and the Cargo
workspace) and recreates the service.

### `upgrade [--version <VER>] [--check] [--rebuild] [-y]`

Upgrades the DeepSeek Harness package (`@deepseek-ai/dsh`) to the latest upstream version:
- Checks npm registry (`registry.npmjs.org` or `registry.npmmirror.com` if using China mirror).
- Updates `DSH_VERSION` in `.env`.
- If the container is currently running, applies the package update live inside the container and restarts the service without rebuilding the entire image.
- With `--rebuild`, forces a rebuild of the local Docker image with the new `DSH_VERSION`.
- `--check` checks for newer versions without applying changes.

### `image`

- `image check [--network]` checks the manifest, GHCR endpoint, and layer CDN DNS.
- `image pull [--mirror ...]` pulls the configured image using the mirror policy.

### `config`

- `config validate` — parse and validate `.env` without Docker.
- `config show` — JSON with secrets and proxy credentials redacted.

### `migrate [--dry-run]`

Copies `users.caddy` to `users.htpasswd`, updates a v1 `:latest` image reference to
`:2`, creates the runtime tree, and prints the next step. The old auth file is kept.

### `self-update [--check]`

Queries GitHub Releases, downloads the current platform asset, verifies
`SHA256SUMS`, and atomically replaces the binary on Unix. On Windows it downloads
the verified archive and asks you to replace the executable manually.

### `completion <shell>`

Generates bash/zsh/fish/elvish/PowerShell completions.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | General failure (including doctor findings) |
| 2 | CLI usage error (clap) |

The CLI currently reports Docker/network/auth failures as general failures with a
specific message. Scripts should inspect the message or use `status --json`.
