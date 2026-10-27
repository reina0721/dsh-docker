# SSH and Git identity

## Persistence

```text
.runtime/home/ssh/        -> /home/node/.ssh
.runtime/home/git/config  -> /home/node/.gitconfig
```

Keys and Git configuration survive container rebuilds. The CLI never prints or logs
private key material.

`ssh generate` and `ssh test` use the host's OpenSSH client (`ssh-keygen`/`ssh`);
`git setup/show` use the host's `git`. The container image also includes
`openssh-client` and Git for `dshctl shell` workflows.

## Generate a key

```bash
dshctl ssh generate work --type ed25519 --comment "you@example.com"
```

- Default name: `id_ed25519`, `id_rsa`, or `id_ecdsa` depending on `--type`.
- Without `--no-passphrase`, `ssh-keygen` prompts for a passphrase on the terminal.
- In unattended scripts, `--no-passphrase` is required; use it only when the runtime
  directory is appropriately protected.
- `--force` replaces an existing pair.

## Import an existing key

```bash
dshctl ssh import ~/.ssh/id_ed25519 --name github
# or with an explicit public key
dshctl ssh import ./deploy_key --public-key ./deploy_key.pub --name deploy
```

The CLI:

- validates that the file looks like a private key and is not a symlink;
- writes it to `.runtime/home/ssh/<name>` with mode 0600;
- copies/derives the public key (0644) when possible;
- warns if an encrypted key prevents automatic public-key derivation.

## Inspect, test, remove

```bash
dshctl ssh list
dshctl ssh list --json
dshctl ssh test github --key work
dshctl ssh remove work
```

`list` shows the name, key type, whether a private key exists, the fingerprint, and
the public line. `test` accepts `github`, `gitlab`, `gitee`, `host`, or `user@host`;
it uses `StrictHostKeyChecking=accept-new` and does not disable host verification.

GitHub prints a successful greeting but exits with status 1; the CLI treats that
specific case as success and asks you to read the message.

## Add the public key to GitHub

1. Run `dshctl ssh list` and copy the `Public key` line.
2. GitHub → Settings → SSH and GPG keys → New SSH key.
3. Test:

   ```bash
   dshctl ssh test github --key work
   ```

4. If using a non-default key for all repositories:

   ```bash
   dshctl git setup --name "Your Name" --email "you@example.com" --key work
   ```

   This sets `core.sshCommand` to
   `ssh -i /home/node/.ssh/work -o IdentitiesOnly=yes` in the container's Git config.

## Multiple keys

Use `dshctl git setup --key <name>` per deployment, or create
`.runtime/home/ssh/config`:

```sshconfig
Host github.com
  IdentityFile ~/.ssh/work
  IdentitiesOnly yes
  AddKeysToAgent yes
```

The file is preserved by backups and is not modified by the CLI.

## Permissions and backups

- `.runtime/home/ssh` is 0700; private keys 0600; public keys 0644.
- `dshctl backup` includes the SSH directory. The resulting archive is a secret.
- Do not commit `.runtime/` or private keys to Git.
- Do not paste private keys into issues or chat.
