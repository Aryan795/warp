use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{ErrorKind, Read as _};
use std::path::PathBuf;
use std::time::Duration;

use instant::Instant;

use crate::terminal::SizeInfo;
use crate::terminal::local_tty::shell::{DirectShellStarter, ShellStarter};
use crate::terminal::local_tty::{PtyOptions, mio_channel};
use crate::terminal::shell::ShellType;
use crate::util::windows::any_powershell_path;

struct CurrentDirGuard(PathBuf);

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).expect("failed to restore working directory");
    }
}

struct SpawnedPtyGuard {
    result: super::PtySpawnResult,
    child: super::PseudoConsoleChild,
}

impl Drop for SpawnedPtyGuard {
    fn drop(&mut self) {
        unsafe {
            self.result.conpty_api.close(self.result.pty_handle);
        }
        let _ = self.result.pipe.disconnect();
        let _ = self.child.kill();
    }
}

#[test]
#[ignore = "requires an extracted Windows TUI package"]
fn packaged_conpty_spawns_powershell_and_reads_output() {
    let package_root = std::env::var_os("WARP_PACKAGED_TUI_ROOT")
        .map(PathBuf::from)
        .expect("WARP_PACKAGED_TUI_ROOT must point to an extracted TUI package");
    let original_dir = std::env::current_dir().expect("failed to read working directory");
    std::env::set_current_dir(&package_root).expect("failed to enter packaged TUI directory");
    let _current_dir_guard = CurrentDirGuard(original_dir);

    let powershell_path = any_powershell_path()
        .cloned()
        .expect("PowerShell is required for the packaged ConPTY smoke test");
    let marker = "WARP_PACKAGED_CONPTY_SMOKE_OK";
    let shell_starter = DirectShellStarter::new_for_test(
        ShellType::PowerShell,
        powershell_path,
        vec![
            OsString::from("-NoLogo"),
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-Command"),
            OsString::from(format!("Write-Output {marker}")),
        ],
    );
    let options = PtyOptions {
        size: SizeInfo::new_without_font_metrics(24, 80),
        window_id: None,
        shell_starter: ShellStarter::Direct(shell_starter),
        start_dir: Some(package_root),
        env_vars: HashMap::new(),
        enable_ssh_wrapper: false,
        reuse_ssh_control_master: false,
        shell_debug_mode: false,
        honor_ps1: false,
        node_version_chip_enabled: false,
        close_fds: false,
    };
    let (event_loop_tx, _event_loop_rx) = mio_channel::channel();
    let spawn_info = super::spawn(options, event_loop_tx).expect("failed to spawn packaged ConPTY");
    let mut spawned = SpawnedPtyGuard {
        result: spawn_info.result,
        child: spawn_info.child,
    };

    let mut poll = mio::Poll::new().expect("failed to create PTY poll");
    poll.registry()
        .register(
            &mut spawned.result.pipe,
            mio::Token(1),
            mio::Interest::READABLE,
        )
        .expect("failed to register packaged ConPTY pipe");
    let mut events = mio::Events::with_capacity(8);
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);

    while Instant::now() < deadline {
        poll.poll(
            &mut events,
            Some(deadline.saturating_duration_since(Instant::now())),
        )
        .expect("failed to poll packaged ConPTY");
        for event in &events {
            if event.token() != mio::Token(1) {
                continue;
            }

            let mut buffer = [0; 4096];
            loop {
                match spawned.result.pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => output.extend_from_slice(&buffer[..bytes_read]),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) => panic!("failed to read packaged ConPTY output: {error}"),
                }
            }
        }

        if String::from_utf8_lossy(&output).contains(marker) {
            return;
        }
    }

    panic!(
        "packaged ConPTY did not produce the expected marker; output: {}",
        String::from_utf8_lossy(&output)
    );
}
