# Prepare a least-privilege SSH account

ServerDesk does not require root and does not install an agent. It connects to
the server with an ordinary SSH account and uses that account's permissions for
the terminal, SFTP, projects, and logs. The Services page is the only feature
that uses `sudo`, and only through fixed, non-interactive commands for the units
you explicitly allow.

Use a dedicated account for ServerDesk. Do not reuse your interactive
administrator account, and do not grant unrestricted sudo.

## What ServerDesk needs

- The account can log in over SSH with a key or password.
- The SSH server offers a host key that you can verify.
- The account can read or write the project and log paths you want to work with.
- For service actions, the account has a passwordless sudo rule for the exact
  `systemctl` command lines listed below.
- `sudo` is installed at `/usr/bin/sudo` and `systemctl` at
  `/usr/bin/systemctl`. ServerDesk uses those absolute paths. A distribution
  that places them elsewhere is not supported by the service actions without a
  code change.

The terminal gives the account whatever shell access it already has. The
account is the security boundary; ServerDesk cannot make an over-privileged
account safe.

## Create the account

Choose a name such as `serverdesk` or `deploy`. The examples use `serverdesk`.

Debian and Ubuntu:

```bash
sudo adduser --disabled-password --gecos "" serverdesk
```

Fedora and RHEL-compatible systems:

```bash
sudo useradd --create-home --shell /bin/bash serverdesk
```

Do not add the account to `wheel`, `sudo`, `docker`, or any other group that
grants broad administrative control. Membership in `adm` is covered under
[file and log access](#file-and-log-access).

## Add an SSH key

Generate a dedicated key on your workstation:

```bash
ssh-keygen -t ed25519 -a 100 -f ~/.ssh/serverdesk_ed25519 -C "serverdesk"
```

Protect it with a passphrase. ServerDesk can store the passphrase with the key
in the OS credential store.

Copy the public key to the server. If you created a password for the account,
`ssh-copy-id` is the simplest option:

```bash
ssh-copy-id -i ~/.ssh/serverdesk_ed25519.pub serverdesk@server.example.com
```

If the account has no password, install the key from an administrative shell:

```bash
sudo install -d -m 700 -o serverdesk -g serverdesk /home/serverdesk/.ssh
echo 'ssh-ed25519 AAAA...your-public-key... serverdesk' \
  | sudo tee /home/serverdesk/.ssh/authorized_keys >/dev/null
sudo chown serverdesk:serverdesk /home/serverdesk/.ssh/authorized_keys
sudo chmod 600 /home/serverdesk/.ssh/authorized_keys
```

Check the permissions on the home directory as well. OpenSSH rejects keys when
the home directory, `.ssh`, or `authorized_keys` is writable by another user:

```bash
sudo chmod go-w /home/serverdesk /home/serverdesk/.ssh
```

Test the account before changing the SSH server configuration:

```bash
ssh -i ~/.ssh/serverdesk_ed25519 -o IdentitiesOnly=yes serverdesk@server.example.com
```

## Harden sshd carefully

Key-only authentication is recommended. Keep an existing root or console
session open while changing SSH configuration so a mistake cannot lock you out.

Create a drop-in configuration file:

```bash
sudoedit /etc/ssh/sshd_config.d/10-serverdesk.conf
```

Example contents:

```text
PubkeyAuthentication yes
PasswordAuthentication no
PermitRootLogin no
```

Validate the configuration and reload the SSH service:

```bash
sudo sshd -t
sudo systemctl reload ssh    # Debian and Ubuntu
sudo systemctl reload sshd   # Fedora and RHEL-compatible systems
```

Do not reload until `sshd -t` succeeds and a second terminal can still log in.

If you disable password authentication, make sure every account you still need
has a working key first. ServerDesk reports a rejected password or key as an
`authentication` error.

## Grant scoped sudo for services

ServerDesk runs service status and actions with:

```text
/usr/bin/sudo -n -- /usr/bin/systemctl show ... -- <unit>
/usr/bin/sudo -n -- /usr/bin/systemctl start -- <unit>
/usr/bin/sudo -n -- /usr/bin/systemctl stop -- <unit>
/usr/bin/sudo -n -- /usr/bin/systemctl restart -- <unit>
```

The `-n` flag means non-interactive. ServerDesk never prompts for a sudo
password, never reads one from the terminal, and never falls back to root or a
different command.

Create one sudoers file for the account:

```bash
sudo visudo -f /etc/sudoers.d/serverdesk
```

Add one rule per unit and action. This example allows read-only status and
start, stop, and restart for `nginx.service`:

```text
# ServerDesk: read status and control exactly this unit.
serverdesk ALL=(root) NOPASSWD: /usr/bin/systemctl show --no-pager --property=Id --property=LoadState --property=ActiveState --property=SubState --property=UnitFileState --property=Description -- nginx.service
serverdesk ALL=(root) NOPASSWD: /usr/bin/systemctl start -- nginx.service
serverdesk ALL=(root) NOPASSWD: /usr/bin/systemctl stop -- nginx.service
serverdesk ALL=(root) NOPASSWD: /usr/bin/systemctl restart -- nginx.service
```

The arguments must match exactly, including `--`, `--no-pager`, and every
`--property` flag. The status command uses `systemctl show`, not
`systemctl status`. Repeat the four lines for each additional unit, replacing
`nginx.service`. Never use a wildcard, `ALL`, a shell, or a generic
`systemctl` rule.

Validate the file:

```bash
sudo visudo -cf /etc/sudoers.d/serverdesk
sudo chown root:root /etc/sudoers.d/serverdesk
sudo chmod 0440 /etc/sudoers.d/serverdesk
```

Test the read-only path from an administrative shell:

```bash
sudo -u serverdesk sudo -n -- /usr/bin/systemctl show --no-pager \
  --property=Id --property=LoadState --property=ActiveState \
  --property=SubState --property=UnitFileState --property=Description \
  -- nginx.service
```

This should print the unit properties without a password prompt. Then open the
Services page in ServerDesk and inspect the unit. Test start, stop, or restart
on a non-critical unit first.

If the rule is missing or does not match exactly, ServerDesk shows the remote
`sudo: a password is required` output and the rule it expects. The action fails;
it does not silently escalate. If you only need status inspection, grant only
the `show` line and leave the action buttons failing with a clear permission
error.

The interactive terminal is separate from this sudo setup. It runs as the SSH
account and has exactly that account's privileges.

## File and log access

ServerDesk never uses sudo for SFTP or logs. Grant access with normal Unix
ownership, groups, or ACLs:

- The account should own or be able to read the Laravel project and its
  `storage/logs` directory. PHP-FPM or the queue worker may run as a different
  user; use a shared group or ACL rather than `chmod 777`.
- System logs are readable only by privileged groups on most distributions.
  On Debian and Ubuntu, `/var/log/auth.log` is normally group `adm`:

  ```bash
  sudo usermod -aG adm serverdesk
  ```

  The account must reconnect before the new group membership applies. Check the
  actual owner and mode first with `ls -l /var/log/auth.log`. If group access is
  not appropriate, grant a read ACL to the specific file instead:

  ```bash
  sudo setfacl -m u:serverdesk:r /var/log/nginx/error.log
  ```

- ServerDesk reads only a fixed list of system logs and `.log` files directly
  inside a Laravel `storage/logs` directory. Do not create broad read access for
  arbitrary files.
- The File Manager's selected root must be a directory the account can access.
  Operations outside that root are rejected, and symlinks that resolve outside
  the canonical root are blocked.

## Verify from ServerDesk

1. Add the server profile with the dedicated account and key.
2. Verify the host key fingerprint through an independent channel.
3. Open **Overview** and confirm identity and monitoring load.
4. Open **Services** and inspect one unit. The inspect action uses the scoped
   `show` rule.
5. Optionally start, stop, or restart a non-critical unit and confirm the
   verified state.
6. Open **Logs** and read a log the account is allowed to access.
7. Open **Terminal** and confirm the expected shell.

If a step fails, see [Troubleshooting](troubleshooting.md#permission-and-sudo-problems).

## Remove access

Revoke the account's access when it is no longer needed:

```bash
sudo rm -f /etc/sudoers.d/serverdesk
sudo userdel -r serverdesk
```

`userdel -r` removes the home directory and the authorized key. Remove the
ServerDesk profile locally as well so the saved credential and trusted host key
are deleted.

## Related documentation

- [Host key verification and recovery](host-keys.md)
- [Security and privacy model](security.md)
- [Troubleshooting](troubleshooting.md)
