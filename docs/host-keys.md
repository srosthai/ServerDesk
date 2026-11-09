# Host key verification and recovery

SSH host keys identify a server. ServerDesk checks the server's host key before
it sends a password, private key, or terminal input. The first connection to a
server requires explicit trust, and a changed key blocks the connection instead
of replacing the saved fingerprint.

This guide explains what ServerDesk displays, how to verify a fingerprint
independently, and how to recover when a key changes legitimately.

## What ServerDesk stores

ServerDesk keeps trusted fingerprints in its local SQLite database, per server
profile and key type. It does not read or write your OpenSSH
`~/.ssh/known_hosts` file. Trusting a key in the `ssh` command does not trust it
in ServerDesk, and the reverse is also true.

Each record contains:

- the SSH key type, such as `ssh-ed25519`;
- an OpenSSH SHA256 fingerprint in the form `SHA256:` followed by 43 base64
  characters;
- when the key was first seen and when it was trusted.

Deleting a server profile deletes its trusted host-key records. Removing a
single fingerprint from the profile leaves the others in place.

## Verify a fingerprint

Get the fingerprint through a channel that does not depend on the SSH connection
you are about to trust. The hosting provider's web console, a recovery console,
or a support channel is usually the right source.

On the server console, print the fingerprint of each host public key:

```bash
for key in /etc/ssh/ssh_host_*_key.pub; do ssh-keygen -lf "$key"; done
```

The output includes the key size, the SHA256 fingerprint, a comment, and the key
type. For example:

```text
256 SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA root@server (ED25519)
```

Match both the key type and the complete fingerprint. The `SHA256:` prefix and
the base64 text are case-sensitive.

If you already have a trusted machine and a trusted network path, you can scan
the server from there:

```bash
ssh-keyscan -t ed25519 server.example.com 2>/dev/null | ssh-keygen -lf -
```

`ssh-keyscan` uses the same network path as the connection you are verifying.
Use it as a convenience, not as your only source when a key change is
suspicious.

Some providers show fingerprints in their control panel. Make sure the panel is
showing the same key type that ServerDesk displays. A server can have separate
Ed25519, ECDSA, and RSA host keys.

## First connection

When a profile has no trusted fingerprints, ServerDesk opens **Verify SSH host
key** before authentication. It shows:

- the server name and host;
- the key type;
- the fingerprint;
- a copy button for the fingerprint.

Compare the values with the independent source. Then:

- **Trust and connect** saves the fingerprint and continues with
  authentication. During a temporary test, the trust lasts only for that test.
- **Reject key** closes the connection without sending credentials.

A connection test follows the same rule. If the test uses an unsaved profile or
edited connection details, accepting the key applies only to that test.
Saving and connecting verifies the key again. The app displays a note when the
trust is temporary.

## What a changed key means

If the server presents a key that does not match the profile's trusted
fingerprints, ServerDesk shows **SSH host key changed** and blocks the
connection before authentication. The dialog shows the saved fingerprint and
the received fingerprint. The saved record is not changed automatically.

A changed key can mean:

- the server was rebuilt, restored from a snapshot, or had its host keys
  regenerated;
- the server is behind a load balancer or DNS record that now points at a
  different machine;
- an attacker is intercepting the connection.

Do not remove trust just to make the dialog go away. Verify the new fingerprint
through an independent channel first.

## Recover from an intentional key change

Use this procedure only after confirming the new fingerprint through the
provider console or another trusted channel.

1. Open **Servers**.
2. Select the fingerprint icon in the profile's row (tooltip **Trusted host
   keys**).
3. Remove every trusted fingerprint for that profile. Removing all entries is
   the simplest recovery when the key type also changed; otherwise a remaining
   entry for another key type can still cause a mismatch.
4. Connect again.
5. Verify the new key type and fingerprint in **Verify SSH host key**.
6. Select **Trust and connect**.

The confirmation dialog states that removing a fingerprint changes only local
trust and does not modify the remote server. The next connection requires
verification again.

### If the change was not intentional

Treat the connection as untrusted:

1. Do not remove the saved fingerprint and do not send credentials.
2. Check the provider's console, recent rebuilds, snapshots, DNS changes, and
   load-balancer configuration.
3. If you suspect compromise, isolate the server, rebuild it from a trusted
   image, rotate its host keys and all credentials, and only then trust the new
   key.
4. If the server is behind a load balancer, trust the key that the balancer
   actually presents and confirm that this is the intended design.

ServerDesk cannot determine whether a remote server is honest. Blocking a
changed key protects against a silent replacement; it cannot prove that a
first-seen key belongs to the intended server. That is why the first trust
decision needs an independent fingerprint check.

## Trusted keys and test connections

The profile stores trust per key type. A successful authentication does not
update or add a fingerprint. A host-key change is never saved automatically,
even when the rest of the connection succeeds.

If you use the same server under two DNS names or two profiles, each profile
verifies the key separately. This is intentional: trust is attached to the
saved profile, not to a global host name.

## Remove trust without changing the server

To make the next connection verify the key again:

1. Open **Servers**.
2. Select the fingerprint icon in the profile's row (tooltip **Trusted host
   keys**).
3. Select **Remove** next to the fingerprint.
4. Confirm **Remove fingerprint**.

This changes only local records. It does not restart `sshd`, regenerate a key,
or modify anything on the remote server. To rotate the actual host keys, use the
server's own administrative procedure and then follow the recovery steps above.

## Related documentation

- [Prepare a least-privilege SSH account](ssh-setup.md)
- [Security and privacy model](security.md)
- [Troubleshooting](troubleshooting.md#host-key-problems)
