use std::path::Path;

use futures_lite::future;

use super::{
    CappedGitOutput, WslGitCommand, build_wslenv, run_git_command, run_git_command_capped,
    translate_for_wsl_unc_cwd,
};

/// Initializes a git repo at `repo_path` with `file_name` staged (but not
/// committed) containing `contents`, so `git show :<file_name>` returns the
/// exact staged blob content with no diff formatting overhead.
fn init_repo_with_staged_file(repo_path: &Path, file_name: &str, contents: &[u8]) {
    future::block_on(async {
        run_git_command(repo_path, &["init", "-q"])
            .await
            .expect("git init");
        std::fs::write(repo_path.join(file_name), contents).expect("write staged file");
        run_git_command(repo_path, &["add", file_name])
            .await
            .expect("git add");
    });
}

/// Translates a git command in `cwd`, asserting that the working directory qualified for the WSL
/// rewrite.
fn translate(args: &[&str], cwd: &str, env: &[(&str, &str)]) -> WslGitCommand {
    translate_for_wsl_unc_cwd(args, Path::new(cwd), env).expect("expected translation")
}

#[test]
fn translates_git_in_unc_cwd() {
    let translated = translate(&["status", "--short"], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
            "--short",
        ]
    );
    assert_eq!(translated.wslenv, "");
}

#[test]
fn does_not_translate_non_unc_cwd() {
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new(r"C:\Users\user\repo"), &[]),
        None
    );
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new("/home/user/repo"), &[]),
        None
    );
}

#[test]
fn rewrites_same_distro_unc_argument_to_linux_path() {
    let translated = translate(
        &["-C", r"\\wsl$\Ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn rewrites_argument_with_case_insensitive_distro_match() {
    let translated = translate(
        &["-C", r"\\wsl$\ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn leaves_other_distro_unc_argument_unchanged() {
    let other = r"\\wsl$\Debian\home\user\other";
    let translated = translate(&["-C", other], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            other,
        ]
    );
}

#[test]
fn build_wslenv_excludes_path_case_insensitively() {
    assert_eq!(
        build_wslenv(&[("PATH", "/usr/bin"), ("GIT_OPTIONAL_LOCKS", "0")]),
        "GIT_OPTIONAL_LOCKS/u"
    );
    assert_eq!(
        build_wslenv(&[("Path", "/usr/bin"), ("GIT_AUTHOR_NAME", "Ada")]),
        "GIT_AUTHOR_NAME/u"
    );
    assert_eq!(build_wslenv(&[("path", "/usr/bin")]), "");
    assert_eq!(build_wslenv(&[]), "");
}

#[test]
fn builds_wslenv_from_env_keys() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_AUTHOR_NAME", "Ada"), ("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(translated.wslenv, "GIT_AUTHOR_NAME/u:GIT_OPTIONAL_LOCKS/u");
}

#[test]
fn omits_wslenv_when_no_env_keys() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[]);

    assert_eq!(translated.wslenv, "");
}

#[test]
fn carries_explicit_path_through_argv() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("PATH", "/usr/local/bin:/usr/bin")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/usr/local/bin:/usr/bin",
            "git",
            "commit",
        ]
    );
    assert_eq!(translated.wslenv, "");
}

#[test]
fn carries_case_insensitive_path_through_argv() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[("Path", "/opt/bin")]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/opt/bin",
            "git",
            "status",
        ]
    );
}

#[test]
fn routes_through_login_shell_when_no_path() {
    let translated = translate(
        &["status"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
        ]
    );
    assert_eq!(translated.wslenv, "GIT_OPTIONAL_LOCKS/u");
}

#[test]
fn capped_command_returns_complete_output_under_budget() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let contents = b"hello world\n".repeat(10);
    init_repo_with_staged_file(repo_dir.path(), "file.txt", &contents);

    let output = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":file.txt"],
        contents.len() + 1,
    ))
    .expect("run_git_command_capped should succeed under budget");

    match output {
        CappedGitOutput::Complete(text) => assert_eq!(text.as_bytes(), contents.as_slice()),
        CappedGitOutput::Exceeded => panic!("expected Complete output under budget"),
    }
}

#[test]
fn capped_command_reports_exceeded_over_budget_without_full_payload() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let contents = vec![b'a'; 10_000];
    init_repo_with_staged_file(repo_dir.path(), "big.txt", &contents);

    let output = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":big.txt"],
        1_000,
    ))
    .expect("run_git_command_capped should succeed even when the budget is exceeded");

    assert!(matches!(output, CappedGitOutput::Exceeded));
}

#[test]
fn capped_command_preserves_git_error_semantics() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    future::block_on(run_git_command(repo_dir.path(), &["init", "-q"])).expect("git init");

    // No such path has ever been staged, so `git show` exits non-zero with no
    // stdout — the capped path must classify this as an error exactly like
    // the unbounded `run_git_command`, not as a successful empty capture.
    let result = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":missing.txt"],
        1_000,
    ));

    assert!(result.is_err());
}
