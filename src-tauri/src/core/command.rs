//! Central construction of remote command lines.
//!
//! Every dynamic command line is built here. User-influenced paths are
//! normalized before they reach this module and are always quoted as one POSIX
//! shell argument. Action names, systemd verbs, and fixed literals are
//! `&'static str` values from closed enums, so arbitrary command text cannot be
//! interpolated.
//!
//! The one interactive shell is the Terminal page. It is an explicit feature,
//! not a command-construction path, and it deliberately sends user keystrokes
//! to the remote shell as the connected SSH account.

/// Fixed binaries used by every service action command.
///
/// The action path always uses non-interactive sudo (`-n`). It never falls back
/// to an interactive password prompt or to direct root execution.
pub const SUDO_BINARY: &str = "/usr/bin/sudo";
pub const SYSTEMCTL_BINARY: &str = "/usr/bin/systemctl";
pub const SERVICE_STATUS_PROPERTIES: &str =
    "Id,LoadState,ActiveState,SubState,UnitFileState,Description";
/// Repeated `--property` flags avoid commas in the sudoers command arguments.
pub const SERVICE_STATUS_PROPERTY_FLAGS: &str = "--property=Id --property=LoadState \
                                                 --property=ActiveState --property=SubState \
                                                 --property=UnitFileState --property=Description";

pub(crate) const LOG_REAL_MARKER: &str = "__SERVERDESK_LOG_REAL__";
pub(crate) const LOG_BEFORE_MARKER: &str = "__SERVERDESK_LOG_BEFORE__";
pub(crate) const LOG_AFTER_MARKER: &str = "__SERVERDESK_LOG_AFTER__";
pub(crate) const LOG_MISSING_MARKER: &str = "__SERVERDESK_LOG_MISSING__";
pub(crate) const LOG_NOT_FILE_MARKER: &str = "__SERVERDESK_LOG_NOT_FILE__";
pub(crate) const LOG_UNREADABLE_MARKER: &str = "__SERVERDESK_LOG_UNREADABLE__";
pub(crate) const LOG_READ_FAILED_MARKER: &str = "__SERVERDESK_LOG_READ_FAILED__";
pub(crate) const LOG_GONE_MARKER: &str = "__SERVERDESK_LOG_GONE__";

/// Quotes one value as a single POSIX shell argument.
///
/// Callers must reject NUL bytes and control characters before building a
/// command. Quoting keeps spaces, quotes, globs, and shell metacharacters
/// usable without letting them escape the argument.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The fixed command used to list immediate child directories.
pub fn browse_directories_command(path: &str) -> String {
    format!(
        "find {} -mindepth 1 -maxdepth 1 \\( -type d -o -xtype d \\) -print0",
        shell_quote(path)
    )
}

/// The fixed command used to probe one candidate project path.
pub fn project_probe_command(path: &str) -> String {
    let path = shell_quote(path);
    format!(
        "if [ -d {path} ]; then echo __directory__; else echo __missing__; fi; \
         if [ -f {path}/artisan ]; then echo artisan; fi; \
         if [ -f {path}/composer.json ]; then echo composer.json; fi; \
         if [ -d {path}/bootstrap ]; then echo bootstrap; fi; \
         if [ -d {path}/storage ]; then echo storage; fi; \
         if [ -f {path}/routes/console.php ]; then echo routes/console.php; fi; \
         if [ -f {path}/app/Console/Kernel.php ]; then echo app/Console/Kernel.php; fi; \
         if [ -f {path}/config/queue.php ]; then echo config/queue.php; fi"
    )
}

/// The fixed command used to read Composer's installed package metadata.
pub fn laravel_package_command(path: &str) -> String {
    let path = shell_quote(path);
    format!(
        "if [ -f {path}/vendor/composer/installed.json ]; then cat {path}/vendor/composer/installed.json; fi"
    )
}

/// The fixed command used to read only the queue connection name from `.env`.
pub fn queue_connection_command(path: &str) -> String {
    let path = shell_quote(path);
    format!("if [ -f {path}/.env ]; then sed -n 's/^QUEUE_CONNECTION=//p' {path}/.env; fi")
}

/// The fixed command used to find a schedule:run cron entry for this path.
pub fn scheduler_cron_command(path: &str) -> String {
    let path = shell_quote(path);
    format!("crontab -l 2>/dev/null | grep -F -- 'schedule:run' | grep -F -- {path} || true")
}

/// The fixed Artisan command for one validated project path and closed action.
pub fn artisan_command(path: &str, argument: &'static str) -> String {
    format!("cd -- {} && php artisan {argument}", shell_quote(path))
}

/// The fixed status command for one validated systemd unit.
pub fn service_status_command(unit: &str) -> String {
    format!(
        "{SUDO_BINARY} -n -- {SYSTEMCTL_BINARY} show --no-pager \
         {SERVICE_STATUS_PROPERTY_FLAGS} -- {unit}"
    )
}

/// The fixed start, stop, or restart command for one validated systemd unit.
pub fn service_action_command(action: &'static str, unit: &str) -> String {
    format!("{SUDO_BINARY} -n -- {SYSTEMCTL_BINARY} {action} -- {unit}")
}

/// Builds the only remote command used for one log tail.
///
/// The path is quoted as one shell argument and the byte count is a fixed
/// numeric bound. The command emits small metadata markers on stderr and the
/// actual bounded tail on stdout.
pub fn log_tail_command(path: &str, byte_limit: usize) -> String {
    let quoted = shell_quote(path);
    format!(
        "log_path={quoted}; \
         if [ ! -e \"$log_path\" ]; then printf '%s\\n' '{LOG_MISSING_MARKER}' >&2; exit 41; fi; \
         if [ ! -f \"$log_path\" ]; then printf '%s\\n' '{LOG_NOT_FILE_MARKER}' >&2; exit 42; fi; \
         resolved=$(readlink -f -- \"$log_path\" 2>/dev/null) || resolved=''; \
         if [ -z \"$resolved\" ]; then printf '%s\\n' '{LOG_MISSING_MARKER}' >&2; exit 41; fi; \
         printf '%s%s\\n' '{LOG_REAL_MARKER}' \"$resolved\" >&2; \
         if [ ! -r \"$resolved\" ]; then printf '%s\\n' '{LOG_UNREADABLE_MARKER}' >&2; exit 43; fi; \
         if ! before=$(stat -c '%d:%i:%s:%Y' -- \"$resolved\" 2>/dev/null); then \
           printf '%s\\n' '{LOG_UNREADABLE_MARKER}' >&2; exit 43; \
         fi; \
         printf '%s%s\\n' '{LOG_BEFORE_MARKER}' \"$before\" >&2; \
         tail -c {byte_limit} -- \"$resolved\"; \
         read_status=$?; \
         if [ \"$read_status\" -ne 0 ]; then \
           printf '%s%s\\n' '{LOG_READ_FAILED_MARKER}' \"$read_status\" >&2; exit 44; \
         fi; \
         if ! after=$(stat -c '%d:%i:%s:%Y' -- \"$resolved\" 2>/dev/null); then \
           printf '%s\\n' '{LOG_GONE_MARKER}' >&2; exit 0; \
         fi; \
         printf '%s%s\\n' '{LOG_AFTER_MARKER}' \"$after\" >&2"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn hostile_path(marker: &std::path::Path) -> String {
        format!(
            "/tmp/serverdesk-fixture; touch {} $(touch {}) `touch {}` #",
            marker.display(),
            marker.display(),
            marker.display()
        )
    }

    fn marker(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("serverdesk-{name}-{nonce}"))
    }

    #[test]
    fn shell_quoting_keeps_metacharacters_inside_one_argument() {
        assert_eq!(shell_quote("/var/www/app"), "'/var/www/app'");
        assert_eq!(
            shell_quote("/var/www/My App's Files"),
            "'/var/www/My App'\\''s Files'"
        );

        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote("a;b $(id) `id` 'q'")))
            .output()
            .expect("run sh");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "a;b $(id) `id` 'q'"
        );
    }

    #[test]
    fn every_dynamic_builder_quotes_the_path() {
        let path = "/var/www/My App's Files; touch /tmp/pwn";
        let quoted = "'/var/www/My App'\\''s Files; touch /tmp/pwn'";

        let commands = [
            browse_directories_command(path),
            project_probe_command(path),
            laravel_package_command(path),
            queue_connection_command(path),
            scheduler_cron_command(path),
            artisan_command(path, "cache:clear"),
            log_tail_command(path, 1024),
        ];

        for command in commands {
            assert!(
                command.contains(quoted),
                "command did not quote the full path: {command}"
            );
        }
    }

    #[test]
    fn hostile_paths_never_execute_injected_commands() {
        let cases = [
            ("browse", marker("browse")),
            ("probe", marker("probe")),
            ("composer", marker("composer")),
            ("queue", marker("queue")),
            ("cron", marker("cron")),
            ("artisan", marker("artisan")),
            ("log", marker("log")),
        ];

        for (name, marker) in &cases {
            let path = hostile_path(marker);
            let command = match *name {
                "browse" => browse_directories_command(&path),
                "probe" => project_probe_command(&path),
                "composer" => laravel_package_command(&path),
                "queue" => queue_connection_command(&path),
                "cron" => scheduler_cron_command(&path),
                "artisan" => artisan_command(&path, "about"),
                "log" => log_tail_command(&path, 1024),
                _ => unreachable!(),
            };

            let _ = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
                .expect("run generated command");
        }

        for (name, marker) in &cases {
            assert!(
                !marker.exists(),
                "the {name} command executed an injected payload: {}",
                marker.display()
            );
        }
    }

    #[test]
    fn service_commands_use_scoped_non_interactive_sudo() {
        assert_eq!(
            service_status_command("nginx.service"),
            "/usr/bin/sudo -n -- /usr/bin/systemctl show --no-pager \
             --property=Id --property=LoadState --property=ActiveState --property=SubState \
             --property=UnitFileState --property=Description -- nginx.service"
        );
        assert_eq!(
            service_action_command("restart", "nginx.service"),
            "/usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service"
        );
    }

    #[test]
    fn artisan_arguments_are_closed_static_values() {
        let command = artisan_command("/var/www/storefront", "queue:restart");
        assert_eq!(
            command,
            "cd -- '/var/www/storefront' && php artisan queue:restart"
        );
    }
}
