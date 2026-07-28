use std::io::Write as _;
use std::path::{Path, PathBuf};
#[allow(clippy::disallowed_types)]
use std::process::Child;
use std::time::Duration;
use std::{fs, thread};

use command::blocking::Command;
use instant::Instant;
use warp_core::channel::Channel;

use super::{
    CURRENT_POINTER_NAME, InstallLayout, InstallLock, LOCK_FILE_NAME, VERSION_LEASES_DIR_NAME,
    VersionDirState, VersionLease, create_unique_staging_dir_with, download_endpoint,
    extract_windows_archive, is_complete_version_dir, is_safe_version_component, point_current_at,
    prune_old_versions, version_dir_state,
};
#[cfg(unix)]
use super::{LOCK_OWNER_FILE_NAME, StagedUpdate, finalize_staged_version, install_update};
#[cfg(windows)]
use super::{PREVIOUS_POINTER_NAME, verify_authenticode, windows_assets_dir};

const BINARY_NAME: &str = "warp-tui-dev";
const HELPER_MODE_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_MODE";
const HELPER_ROOT_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_ROOT";
const HELPER_VERSION_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_VERSION";
const HELPER_READY_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_READY";
const HELPER_RELEASE_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_RELEASE";

fn temp_root(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("warp-tui-autoupdate-{name}-"))
        .tempdir()
        .unwrap()
}

fn write_zip(path: &Path, entries: &[(&str, &str)]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, contents) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(contents.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn version_directory_names_are_single_safe_components() {
    for valid in ["v0.2026.07.28.12.00.dev_00", "preview-1", "A"] {
        assert!(is_safe_version_component(valid), "{valid}");
    }
    for invalid in [
        "",
        ".",
        "..",
        "v1..dev",
        "../v1",
        "nested/v1",
        "nested\\v1",
        "version:stream",
        "CON",
        "trailing.",
        "contains space",
    ] {
        assert!(!is_safe_version_component(invalid), "{invalid}");
    }
}

#[test]
fn extracts_exact_windows_archive_shape() {
    let root = temp_root("windows-zip");
    let archive_path = root.path().join("warp-tui.zip");
    write_zip(
        &archive_path,
        &[
            (BINARY_NAME, "binary"),
            ("conpty.dll", "conpty"),
            ("x64/OpenConsole.exe", "console"),
            ("resources/marker", "resource"),
            ("resources/nested/value", "nested"),
        ],
    );
    let payload_dir = root.path().join("payload with spaces");

    extract_windows_archive(&archive_path, &payload_dir, BINARY_NAME, "x64").unwrap();

    assert_eq!(
        fs::read_to_string(payload_dir.join(BINARY_NAME)).unwrap(),
        "binary"
    );
    assert_eq!(
        fs::read_to_string(payload_dir.join("x64/OpenConsole.exe")).unwrap(),
        "console"
    );
    assert_eq!(
        fs::read_to_string(payload_dir.join("resources/nested/value")).unwrap(),
        "nested"
    );
}

#[test]
fn rejects_invalid_windows_archives() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "missing-pty",
            &[
                (BINARY_NAME, "binary"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/marker", "resource"),
            ],
        ),
        (
            "unexpected",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/marker", "resource"),
                ("d3dcompiler_47.dll", "unexpected"),
            ],
        ),
        (
            "traversal",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/../escape", "escape"),
            ],
        ),
        (
            "case-collision",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/marker", "resource"),
                ("resources/MARKER", "collision"),
            ],
        ),
        (
            "alternate-data-stream",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/marker:payload", "stream"),
            ],
        ),
        (
            "reserved-name",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/CON.txt", "reserved"),
            ],
        ),
        (
            "trailing-dot",
            &[
                (BINARY_NAME, "binary"),
                ("conpty.dll", "conpty"),
                ("x64/OpenConsole.exe", "console"),
                ("resources/marker.", "normalized"),
            ],
        ),
    ];

    for (name, entries) in cases {
        let root = temp_root(name);
        let archive_path = root.path().join("warp-tui.zip");
        write_zip(&archive_path, entries);

        let error = extract_windows_archive(
            &archive_path,
            &root.path().join("payload"),
            BINARY_NAME,
            "x64",
        )
        .unwrap_err();

        assert!(!format!("{error:#}").is_empty(), "{name}");
        assert!(!root.path().join("escape").exists(), "{name}");
    }
}

fn layout(root: &Path, running_version: &str) -> InstallLayout {
    InstallLayout {
        root: root.to_path_buf(),
        versions_dir: root.join("versions"),
        current_pointer: root.join(CURRENT_POINTER_NAME),
        running_version_dir: root.join("versions").join(running_version),
        binary_name: BINARY_NAME.to_owned(),
    }
}

fn create_complete_version(root: &Path, version: &str, contents: &str) -> PathBuf {
    let version_dir = root.join("versions").join(version);
    fs::create_dir_all(version_dir.join("resources")).unwrap();
    #[cfg(not(windows))]
    fs::write(version_dir.join(BINARY_NAME), contents).unwrap();
    #[cfg(windows)]
    {
        let signed_executable = signed_windows_executable();
        let assets_dir = version_dir.join(windows_assets_dir());
        fs::create_dir_all(&assets_dir).unwrap();
        fs::copy(&signed_executable, version_dir.join(BINARY_NAME)).unwrap();
        fs::copy(&signed_executable, version_dir.join("conpty.dll")).unwrap();
        fs::copy(&signed_executable, assets_dir.join("OpenConsole.exe")).unwrap();
    }
    fs::write(version_dir.join("resources").join("marker"), contents).unwrap();
    version_dir
}

#[cfg(windows)]
fn signed_windows_executable() -> PathBuf {
    let mut candidates = Vec::new();
    for root in ["ProgramW6432", "ProgramFiles"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
    {
        candidates.push(root.join("PowerShell/7/pwsh.exe"));
        candidates.push(root.join("dotnet/dotnet.exe"));
    }
    if let Some(system_root) = std::env::var_os("SystemRoot").map(PathBuf::from) {
        candidates.push(system_root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file() && verify_authenticode(candidate).is_ok())
        .expect(
            "Windows tests require an installed executable with an embedded Authenticode signature",
        )
}

fn lease_path(root: &Path, version: &str) -> PathBuf {
    root.join(VERSION_LEASES_DIR_NAME)
        .join(format!("{version}.lock"))
}

fn wait_for_contents(path: &Path, expected: &str, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read_to_string(path).is_ok_and(|contents| contents == expected) {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("helper exited before writing {expected:?} to {path:?}: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected:?} in {path:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_helper(
    mode: &str,
    root: &Path,
    version: &str,
    ready: &Path,
    release: Option<&Path>,
) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("lease_process_helper")
        .arg("--nocapture")
        .env(HELPER_MODE_ENV, mode)
        .env(HELPER_ROOT_ENV, root)
        .env(HELPER_VERSION_ENV, version)
        .env(HELPER_READY_ENV, ready);
    if let Some(release) = release {
        command.env(HELPER_RELEASE_ENV, release);
    }
    command.spawn().unwrap()
}

#[test]
fn detects_managed_install_layout() {
    let exe = Path::new("/home/user/.warp/tui/versions/v0.2026.01.01.00.00.dev_00/warp-tui-dev");
    let layout = InstallLayout::from_canonical_exe_path(exe).unwrap();

    assert_eq!(layout.root, Path::new("/home/user/.warp/tui"));
    assert_eq!(
        layout.versions_dir,
        Path::new("/home/user/.warp/tui/versions")
    );
    assert_eq!(
        layout.current_pointer,
        Path::new("/home/user/.warp/tui/current")
    );
    assert_eq!(
        layout.running_version_dir,
        Path::new("/home/user/.warp/tui/versions/v0.2026.01.01.00.00.dev_00")
    );
    assert_eq!(layout.binary_name, BINARY_NAME);
}

#[test]
fn rejects_unmanaged_exe_paths() {
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/home/user/.warp/tui/warp-tui-dev")),
        None
    );
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/repo/target/debug/warp-tui-dev")),
        None
    );
    assert!(
        VersionLease::acquire_for_current_process()
            .unwrap()
            .is_none()
    );
}

#[test]
fn uses_channel_specific_download_endpoints() {
    assert_eq!(
        download_endpoint(Channel::Stable),
        "/download/agent-cli/artifact"
    );
    assert_eq!(
        download_endpoint(Channel::Preview),
        "/download/agent-cli-preview/artifact"
    );
    assert_eq!(
        download_endpoint(Channel::Dev),
        "/download/agent-cli-dev/artifact"
    );
}

#[test]
fn complete_versions_require_real_binary_and_resources() {
    let root = temp_root("complete");
    let layout = layout(root.path(), "A");
    let version_dir = create_complete_version(root.path(), "A", "original");
    assert!(is_complete_version_dir(&layout, &version_dir));
    assert_eq!(
        version_dir_state(&layout, &version_dir).unwrap(),
        VersionDirState::Complete
    );

    fs::remove_dir_all(version_dir.join("resources")).unwrap();
    assert_eq!(
        version_dir_state(&layout, &version_dir).unwrap(),
        VersionDirState::Invalid
    );
    #[cfg(not(windows))]
    assert_eq!(
        fs::read_to_string(version_dir.join(BINARY_NAME)).unwrap(),
        "original"
    );
}

#[test]
fn staging_allocation_skips_existing_directory_without_reusing_it() {
    let root = temp_root("staging-collision");
    let versions_dir = root.path().join("versions");
    fs::create_dir(&versions_dir).unwrap();
    let stale = versions_dir.join(".staging-stale");
    let fresh = versions_dir.join(".staging-fresh");
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("marker"), "stale").unwrap();
    let mut candidates = [stale.clone(), fresh.clone()].into_iter();

    let allocated = futures::executor::block_on(create_unique_staging_dir_with(|| {
        candidates.next().unwrap()
    }))
    .unwrap();

    assert_eq!(allocated, fresh);
    assert_eq!(fs::read_to_string(stale.join("marker")).unwrap(), "stale");
    assert!(allocated.is_dir());
}

#[test]
fn managed_version_lease_creates_stable_marker() {
    let root = temp_root("lease");
    let version = "v0.2026.07.22.18.00.dev_00";
    let layout = layout(root.path(), version);
    create_complete_version(root.path(), version, "A");

    let lease = VersionLease::acquire(&layout).unwrap();
    assert!(lease_path(root.path(), version).is_file());
    assert!(layout.running_version_dir.is_dir());
    drop(lease);
    assert!(lease_path(root.path(), version).is_file());
}

#[cfg(unix)]
#[test]
fn finalized_unlaunched_version_is_marked_and_reclaimed() {
    let root = temp_root("finalized-marker");
    let layout = layout(root.path(), "A");
    create_complete_version(root.path(), "A", "A");
    point_current_at(&layout, "A").unwrap();

    let staging_dir = root.path().join("versions/.staging-B");
    let payload_dir = staging_dir.join("payload");
    fs::create_dir_all(payload_dir.join("resources")).unwrap();
    fs::write(payload_dir.join(BINARY_NAME), "B").unwrap();
    fs::write(payload_dir.join("resources/marker"), "B").unwrap();
    let staged = StagedUpdate {
        staging_dir,
        payload_dir,
    };
    let version_dir = root.path().join("versions/B");

    finalize_staged_version(&layout, "B", staged, &version_dir).unwrap();
    assert!(lease_path(root.path(), "B").is_file());

    create_complete_version(root.path(), "C", "C");
    point_current_at(&layout, "C").unwrap();
    prune_old_versions(&layout, "C");
    assert!(!version_dir.exists());
}

#[cfg(unix)]
#[test]
fn completed_version_is_reused_and_invalid_version_is_not_replaced() {
    let root = temp_root("immutable");
    create_complete_version(root.path(), "A", "running");
    let target = create_complete_version(root.path(), "C", "original");
    let layout = layout(root.path(), "A");
    point_current_at(&layout, "A").unwrap();

    futures::executor::block_on(install_update(layout.clone(), "C".to_owned())).unwrap();
    assert_eq!(
        fs::read_to_string(target.join(BINARY_NAME)).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(target.join("resources/marker")).unwrap(),
        "original"
    );

    fs::remove_dir_all(&target).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join(BINARY_NAME), "partial").unwrap();
    point_current_at(&layout, "A").unwrap();
    let error =
        futures::executor::block_on(install_update(layout.clone(), "C".to_owned())).unwrap_err();
    assert!(format!("{error:#}").contains("refusing to replace incomplete or invalid"));
    assert_eq!(
        fs::read_to_string(target.join(BINARY_NAME)).unwrap(),
        "partial"
    );
    assert!(super::current_points_at(&layout, "A"));
}

#[test]
fn live_versions_are_retained_and_reclaimed_after_exit() {
    let root = temp_root("gc");
    create_complete_version(root.path(), "A", "A");
    create_complete_version(root.path(), "B", "B");
    create_complete_version(root.path(), "C", "C");
    create_complete_version(root.path(), "legacy", "legacy");
    let layout = layout(root.path(), "C");
    point_current_at(&layout, "C").unwrap();

    let a_ready = root.path().join("a-ready");
    let a_release = root.path().join("a-release");
    let b_ready = root.path().join("b-ready");
    let b_release = root.path().join("b-release");
    let mut a = spawn_helper("hold-lease", root.path(), "A", &a_ready, Some(&a_release));
    let mut b = spawn_helper("hold-lease", root.path(), "B", &b_ready, Some(&b_release));
    wait_for_contents(&a_ready, "locked", &mut a);
    wait_for_contents(&b_ready, "locked", &mut b);

    prune_old_versions(&layout, "C");
    assert!(root.path().join("versions/A").is_dir());
    assert!(root.path().join("versions/B").is_dir());
    assert!(root.path().join("versions/C").is_dir());
    assert!(root.path().join("versions/legacy").is_dir());

    fs::write(&a_release, "").unwrap();
    assert!(a.wait().unwrap().success());
    prune_old_versions(&layout, "C");
    assert!(!root.path().join("versions/A").exists());
    assert!(lease_path(root.path(), "A").is_file());
    assert!(root.path().join("versions/B").is_dir());

    fs::write(&b_release, "").unwrap();
    assert!(b.wait().unwrap().success());
    prune_old_versions(&layout, "C");
    assert!(!root.path().join("versions/B").exists());
    assert!(root.path().join("versions/C").is_dir());
    assert!(root.path().join("versions/legacy").is_dir());
}

#[test]
fn current_version_is_rechecked_before_gc_deletion() {
    let root = temp_root("gc-current");
    create_complete_version(root.path(), "A", "A");
    create_complete_version(root.path(), "C", "C");
    fs::create_dir_all(root.path().join(VERSION_LEASES_DIR_NAME)).unwrap();
    fs::write(lease_path(root.path(), "A"), "").unwrap();
    let layout = layout(root.path(), "C");
    point_current_at(&layout, "A").unwrap();

    prune_old_versions(&layout, "C");
    assert!(root.path().join("versions/A").is_dir());
}

#[test]
fn startup_fails_closed_if_gc_wins_the_lease_race() {
    let root = temp_root("gc-wins");
    let version_dir = create_complete_version(root.path(), "A", "A");
    fs::create_dir_all(root.path().join(VERSION_LEASES_DIR_NAME)).unwrap();
    let lease_path = lease_path(root.path(), "A");
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&lease).unwrap();

    let ready = root.path().join("race-result");
    let mut child = spawn_helper("attempt-lease", root.path(), "A", &ready, None);
    wait_for_contents(&ready, "starting", &mut child);
    fs::remove_dir_all(version_dir).unwrap();
    fs4::fs_std::FileExt::unlock(&lease).unwrap();
    drop(lease);

    wait_for_contents(&ready, "error", &mut child);
    assert!(child.wait().unwrap().success());
}

#[cfg(windows)]
#[test]
fn authenticode_rejects_unsigned_files() {
    verify_authenticode(&signed_windows_executable()).unwrap();

    let root = temp_root("unsigned");
    let unsigned = root.path().join("unsigned.exe");
    fs::write(&unsigned, "unsigned").unwrap();
    assert!(verify_authenticode(&unsigned).is_err());
}

#[cfg(windows)]
#[test]
fn windows_activation_records_previous_version_atomically() {
    let root = temp_root("windows-pointers");
    let layout = layout(root.path(), "A");
    create_complete_version(root.path(), "A", "A");
    create_complete_version(root.path(), "B", "B");
    fs::write(&layout.current_pointer, "A").unwrap();

    point_current_at(&layout, "B").unwrap();

    assert_eq!(fs::read_to_string(&layout.current_pointer).unwrap(), "B");
    assert_eq!(
        fs::read_to_string(root.path().join(PREVIOUS_POINTER_NAME)).unwrap(),
        "A"
    );
}

#[cfg(windows)]
#[test]
fn windows_gc_retains_rollback_version() {
    let root = temp_root("windows-rollback-gc");
    let layout = layout(root.path(), "C");
    for version in ["A", "B", "C"] {
        create_complete_version(root.path(), version, version);
        fs::create_dir_all(root.path().join(VERSION_LEASES_DIR_NAME)).unwrap();
        fs::write(lease_path(root.path(), version), "").unwrap();
    }
    fs::write(&layout.current_pointer, "C").unwrap();
    fs::write(root.path().join(PREVIOUS_POINTER_NAME), "B").unwrap();

    prune_old_versions(&layout, "C");

    assert!(!root.path().join("versions/A").exists());
    assert!(root.path().join("versions/B").exists());
    assert!(root.path().join("versions/C").exists());
}

#[cfg(windows)]
#[test]
fn windows_install_lock_is_compatible_with_installer_file_sharing() {
    use std::os::windows::fs::OpenOptionsExt as _;

    let root = temp_root("windows-install-lock");
    let lock_path = root.path().join(LOCK_FILE_NAME);
    let installer_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(&lock_path)
        .unwrap();
    assert!(InstallLock::acquire(root.path()).unwrap().is_none());
    drop(installer_lock);

    let updater_lock = InstallLock::acquire(root.path()).unwrap().unwrap();
    assert!(lock_path.is_file());
    assert!(
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&lock_path)
            .is_err()
    );
    drop(updater_lock);

    assert!(lock_path.is_file());
    let _installer_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&lock_path)
        .unwrap();
}

#[cfg(unix)]
#[test]
fn directory_install_lock_is_cross_process_and_token_owned() {
    let root = temp_root("install-lock");
    let ready = root.path().join("lock-ready");
    let release = root.path().join("lock-release");
    let mut child = spawn_helper(
        "hold-install-lock",
        root.path(),
        "unused",
        &ready,
        Some(&release),
    );
    wait_for_contents(&ready, "locked", &mut child);
    assert!(InstallLock::acquire(root.path()).unwrap().is_none());

    fs::write(&release, "").unwrap();
    assert!(child.wait().unwrap().success());
    let lock = InstallLock::acquire(root.path()).unwrap().unwrap();
    fs::write(
        root.path().join(LOCK_FILE_NAME).join(LOCK_OWNER_FILE_NAME),
        "successor",
    )
    .unwrap();
    drop(lock);
    assert!(root.path().join(LOCK_FILE_NAME).is_dir());
}

#[cfg(unix)]
#[test]
fn install_lock_migrates_stale_legacy_file_and_directory() {
    for representation in ["file", "directory"] {
        let root = temp_root(representation);
        let lock_path = root.path().join(LOCK_FILE_NAME);
        if representation == "file" {
            fs::write(&lock_path, "legacy").unwrap();
        } else {
            fs::create_dir(&lock_path).unwrap();
            fs::write(lock_path.join(LOCK_OWNER_FILE_NAME), "stale").unwrap();
        }

        let lock = InstallLock::acquire_with_stale_age(root.path(), Duration::ZERO)
            .unwrap()
            .unwrap();
        assert!(lock_path.is_dir());
        assert_ne!(
            fs::read_to_string(lock_path.join(LOCK_OWNER_FILE_NAME)).unwrap(),
            "stale"
        );
        drop(lock);
        assert!(!lock_path.exists());
    }
}

#[cfg(unix)]
#[test]
fn fresh_legacy_install_lock_is_contention() {
    let root = temp_root("fresh-legacy-lock");
    fs::write(root.path().join(LOCK_FILE_NAME), "legacy").unwrap();
    assert!(InstallLock::acquire(root.path()).unwrap().is_none());
}

#[test]
fn lease_process_helper() {
    let Ok(mode) = std::env::var(HELPER_MODE_ENV) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(HELPER_ROOT_ENV).unwrap());
    let version = std::env::var(HELPER_VERSION_ENV).unwrap();
    let ready = PathBuf::from(std::env::var_os(HELPER_READY_ENV).unwrap());

    match mode.as_str() {
        "hold-lease" => {
            let lease = VersionLease::acquire(&layout(&root, &version)).unwrap();
            fs::write(&ready, "locked").unwrap();
            let release = PathBuf::from(std::env::var_os(HELPER_RELEASE_ENV).unwrap());
            while !release.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            drop(lease);
        }
        "attempt-lease" => {
            fs::write(&ready, "starting").unwrap();
            let result = VersionLease::acquire(&layout(&root, &version));
            fs::write(&ready, if result.is_ok() { "acquired" } else { "error" }).unwrap();
        }
        "hold-install-lock" => {
            let lock = InstallLock::acquire(&root).unwrap().unwrap();
            fs::write(&ready, "locked").unwrap();
            let release = PathBuf::from(std::env::var_os(HELPER_RELEASE_ENV).unwrap());
            while !release.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            drop(lock);
        }
        mode => panic!("unknown helper mode {mode}"),
    }
}
