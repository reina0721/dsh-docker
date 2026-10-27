# Authentication

The Rust gateway uses HTTP Basic Authentication with bcrypt hashes.

Canonical file:

```text
.runtime/auth/users.htpasswd
```

The format is the same `username bcrypt-hash` format Caddy used, so a v1
`users.caddy` file can be migrated without re-hashing passwords.

Create or update a user:

```bash
dshctl user add alice
```

List users:

```bash
dshctl user list
```

Remove a user:

```bash
dshctl user remove alice
```

Passwords are never passed in process arguments. The CLI hashes locally with
Rust and writes the file atomically; the running gateway reloads it without a
container restart.

The file contains password hashes, not plaintext. It is mounted into the
container read-only. Backups that include `.runtime/` contain these hashes and
must be protected like credentials.

SSH/Git credentials remain under `.runtime/home/`.
