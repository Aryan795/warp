use std::path::PathBuf;

/// Env var Warp sets on the re-exec'd host process so later calls to
/// [`flatpak_app_id`] still recognize a Flatpak origin, since `/.flatpak-info`
/// no longer exists once the process is running outside the sandbox's mount
/// namespace.
const FLATPAK_APP_ID_ENV_VAR: &str = "WARP_FLATPAK_APP_ID";

/// Env var that opts out of the host re-exec, keeping Warp inside the
/// sandbox. Mirrors Zed's `ZED_FLATPAK_NO_ESCAPE`.
const NO_ESCAPE_ENV_VAR: &str = "WARP_FLATPAK_NO_ESCAPE";

/// Env vars forwarded to the host-side process. `flatpak-spawn --host` does
/// not inherit the caller's environment by default, so anything missing
/// here won't be visible to the re-exec'd process or the shells it later
/// spawns.
///
/// Deliberately excluded: anything that points back into the sandbox, such
/// as `LD_LIBRARY_PATH`, `GTK_PATH`, and `XDG_DATA_DIRS`. Forwarding those
/// broke Zed's embedded terminal (dynamic linker warnings on every command
/// run on the host); see zed-industries/zed#53129.
const FORWARDED_ENV_VARS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "DBUS_SESSION_BUS_ADDRESS",
    "XAUTHORITY",
    "HOME",
    "USER",
    "LOGNAME",
    "TERM",
    "LANG",
    "LC_ALL",
];

/// If Warp is running inside a Flatpak sandbox, re-execs the real binary on
/// the host via `flatpak-spawn --host` and never returns.
///
/// Warp is a terminal: the shells and tools it launches need the user's real
/// filesystem, network namespace, and `sudo`, none of which make sense
/// behind a sandbox. Rather than punch a sandbox hole for every terminal
/// feature -- the approach Zed tried and abandoned as unmaintainable in
/// zed-industries/zed#10754 and #11949 -- this exits the sandbox entirely at
/// startup, mirroring Zed's shipped approach in zed-industries/zed#12006.
///
/// Must run before anything else touches the PTY, GPU, or network, since all
/// of those behave differently -- or not at all -- inside the sandbox.
pub fn maybe_reexec_on_flatpak_host() {
    if std::env::var_os(NO_ESCAPE_ENV_VAR).is_some() {
        return;
    }
    let Some(info) = read_flatpak_info() else {
        return;
    };
    let Ok(current_exe) = std::env::current_exe() else {
        log::warn!("Detected Flatpak sandbox but could not resolve current executable path");
        return;
    };
    // `/app` inside the sandbox is a bind mount of `info.app_path` on the
    // host, so rewriting the prefix gives flatpak-spawn a path it can find
    // outside the sandbox's mount namespace.
    let Ok(relative_to_app) = current_exe.strip_prefix("/app") else {
        log::warn!(
            "Detected Flatpak sandbox but executable {current_exe:?} is not under /app; \
             skipping host re-exec"
        );
        return;
    };
    let host_binary = PathBuf::from(&info.app_path).join(relative_to_app);

    log::info!(
        "Detected Flatpak sandbox ({}); re-execing {host_binary:?} on the host",
        info.app_id
    );

    let mut command = command::blocking::Command::new("flatpak-spawn");
    command.arg("--host");
    for var in FORWARDED_ENV_VARS {
        if let Ok(value) = std::env::var(var) {
            command.arg(format!("--env={var}={value}"));
        }
    }
    command.arg(format!("--env={FLATPAK_APP_ID_ENV_VAR}={}", info.app_id));
    command.arg(&host_binary);
    command.args(std::env::args_os().skip(1));

    // Replace this process outright via execvp: there's nothing useful left
    // to do inside the sandbox, and leaving the launcher process around
    // would just add a zombie next to the real, host-side Warp.
    use command::unix::CommandExt as _;
    let err = command.exec();
    log::error!(
        "Failed to re-exec Warp on the host via flatpak-spawn ({err:#}); continuing inside the \
         sandbox, where terminal features will not behave like a normal host install"
    );
}

/// Returns the Flatpak app ID that launched this process, whether Warp is
/// still inside the sandbox (read from `/.flatpak-info`) or already
/// re-exec'd onto the host (read from the env var set before re-execing).
///
/// Used by autoupdate to route Flatpak installs to `flatpak update` instead
/// of falling through to a package-manager command that would have no
/// effect on a Flatpak install.
pub fn flatpak_app_id() -> Option<String> {
    std::env::var(FLATPAK_APP_ID_ENV_VAR)
        .ok()
        .or_else(|| read_flatpak_info().map(|info| info.app_id))
}

#[derive(Debug, PartialEq, Eq)]
struct FlatpakInfo {
    app_id: String,
    /// Host filesystem path bind-mounted at `/app` inside the sandbox.
    app_path: String,
}

fn read_flatpak_info() -> Option<FlatpakInfo> {
    let contents = std::fs::read_to_string("/.flatpak-info").ok()?;
    parse_flatpak_info(&contents)
}

/// Parses the two `/.flatpak-info` keyfile fields Warp needs, rather than
/// pulling in a full keyfile parser for them.
fn parse_flatpak_info(contents: &str) -> Option<FlatpakInfo> {
    let mut app_id = None;
    let mut app_path = None;
    let mut section = "";
    for line in contents.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (section, key.trim()) {
            ("Application", "name") => app_id = Some(value.trim().to_owned()),
            ("Instance", "app-path") => app_path = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    Some(FlatpakInfo {
        app_id: app_id?,
        app_path: app_path?,
    })
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
