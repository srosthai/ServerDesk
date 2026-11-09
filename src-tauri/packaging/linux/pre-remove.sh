#!/bin/sh
# Debian and RPM pre-remove scriptlet.
#
# Package upgrades must keep application data. A real uninstall runs the
# application's purge mode as each user who has a ServerDesk data directory,
# which removes keyring credentials and the local SQLite database. The purge
# is deliberately fail-safe: if the OS credential store cannot be reached,
# the data directory is kept and the purge can be retried later.

set -u

case "${1:-0}" in
    0|remove|purge)
        ;;
    *)
        exit 0
        ;;
esac

purge_user() {
    home="$1"
    user="$2"

    data_dir="$home/.local/share/com.serverdesk.app"
    [ -d "$data_dir" ] || return 0

    uid="$(id -u "$user" 2>/dev/null)" || return 0

    if [ "$user" = "root" ]; then
        HOME="$home" \
        XDG_DATA_HOME="$home/.local/share" \
        XDG_RUNTIME_DIR="/run/user/$uid" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
            /usr/bin/serverdesk --purge-data >/dev/null 2>&1 || true
    elif command -v runuser >/dev/null 2>&1; then
        runuser -u "$user" -- env \
            HOME="$home" \
            XDG_DATA_HOME="$home/.local/share" \
            XDG_RUNTIME_DIR="/run/user/$uid" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
            /usr/bin/serverdesk --purge-data >/dev/null 2>&1 || true
    fi
}

for home in /home/* /root; do
    [ -d "$home" ] || continue
    purge_user "$home" "$(basename "$home")"
done

exit 0
