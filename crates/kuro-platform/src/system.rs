//! Facts about the running system that crash reports print. The OS sources (`/etc/os-release`,
//! `/proc`, `XDG_*` variables; the registry on Windows, `windows::system`) are read only here, so no other crate names
//! them.

use std::env;
use std::fs;

const UNKNOWN: &str = "unknown";

/// `(key, value)` pairs for the report keys `os`, `kernel`, `desktop`, `session` and `locale`.
/// A value whose source is missing is `unknown`.
pub fn facts() -> Vec<(&'static str, String)> {
    vec![
        ("os", os_name()),
        ("kernel", kernel()),
        ("desktop", or_unknown(env_value("XDG_CURRENT_DESKTOP"))),
        ("session", or_unknown(env_value("XDG_SESSION_TYPE"))),
        ("locale", or_unknown(["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(env_value).or_else(system_locale))),
    ]
}

/// The locale the system reports when no variable names one (Windows), as `en-US`.
#[allow(unreachable_code)]
fn system_locale() -> Option<String> {
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return crate::windows::system::locale();
    None
}

/// `PRETTY_NAME` from `/etc/os-release`, else `ID`, else `unknown`. Windows: its product name and version.
#[allow(unreachable_code)]
fn os_name() -> String {
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return or_unknown(crate::windows::system::os_name());
    let text = fs::read_to_string("/etc/os-release").unwrap_or_default();
    or_unknown(os_release_value(&text, "PRETTY_NAME").or_else(|| os_release_value(&text, "ID")))
}

/// The unquoted value of `key` in os-release text. Empty values count as missing.
fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix('=')?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// The `uname -sr` text, from `/proc/sys/kernel`. Windows: `Windows NT` with its version and build.
#[allow(unreachable_code)]
fn kernel() -> String {
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return or_unknown(crate::windows::system::kernel());
    let read = |name: &str| fs::read_to_string(format!("/proc/sys/kernel/{name}")).map(|s| s.trim().to_owned());
    match (read("ostype"), read("osrelease")) {
        (Ok(sys), Ok(release)) => format!("{sys} {release}"),
        _ => UNKNOWN.to_owned(),
    }
}

/// An environment variable, where an empty value counts as unset.
fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

fn or_unknown(value: Option<String>) -> String {
    value.unwrap_or_else(|| UNKNOWN.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_prefers_pretty_name_and_unquotes() {
        let text = "NAME=\"CachyOS Linux\"\nID=cachyos\nPRETTY_NAME=\"CachyOS Linux\"\nID_LIKE=arch\n";
        assert_eq!(os_release_value(text, "PRETTY_NAME").as_deref(), Some("CachyOS Linux"));
        assert_eq!(os_release_value(text, "ID").as_deref(), Some("cachyos"));
        assert_eq!(os_release_value("ID_LIKE=arch\n", "ID"), None);
        assert_eq!(os_release_value("PRETTY_NAME=\"\"\n", "PRETTY_NAME"), None);
    }

    #[test]
    fn facts_carry_every_system_key_in_order() {
        let keys: Vec<&str> = facts().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["os", "kernel", "desktop", "session", "locale"]);
    }
}
