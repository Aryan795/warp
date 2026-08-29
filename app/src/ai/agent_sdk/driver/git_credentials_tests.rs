use std::io::Write as _;
use std::process::Stdio;

use command::blocking::Command;

use super::*;

fn gitlab_credential(id: &str, project_path: &str, token: &str) -> GitCredential {
    GitCredential {
        id: id.to_string(),
        instance_uid: Some("instance-uid".to_string()),
        installation_uid: Some(format!("installation-{id}")),
        scheme: "https".to_string(),
        host: "gitlab.example.com".to_string(),
        port: Some(8443),
        relative_url_prefix: "gitlab".to_string(),
        project_paths: vec![project_path.to_string()],
        token: token.to_string(),
        username: Some("oauth2".to_string()),
        email: Some(format!("{id}@example.com")),
    }
}

fn github_credential() -> GitCredential {
    GitCredential {
        id: "github".to_string(),
        instance_uid: None,
        installation_uid: None,
        scheme: "https".to_string(),
        host: "github.com".to_string(),
        port: None,
        relative_url_prefix: String::new(),
        project_paths: vec!["warpdotdev/warp".to_string()],
        token: "github-token".to_string(),
        username: Some("warp-agent[bot]".to_string()),
        email: None,
    }
}

fn git_credential_from_store(store: &Path, project_path: &str) -> String {
    let mut child = Command::new("git")
        .args(["credential-store", "--file", store.to_str().unwrap(), "get"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.as_mut().unwrap(),
        "protocol=https\nhost=gitlab.example.com:8443\npath=gitlab/{project_path}.git\n\n"
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn project_path_normalization_accepts_optional_git_suffix() {
    assert_eq!(
        normalized_project_path("/platform/backend.git/").unwrap(),
        "platform/backend"
    );
    assert_eq!(
        project_key("Platform/Backend.GIT").unwrap(),
        "Platform/Backend"
    );
}

#[test]
fn project_path_normalization_rejects_unsafe_segments() {
    for invalid in ["", "group//repo", "group/../repo", r"group\repo"] {
        assert!(normalized_project_path(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn explicit_clone_url_preserves_port_prefix_and_has_no_token() {
    let credential = gitlab_credential("one", "platform/backend.git", "secret-token");

    let clone_url = clone_url_for_path(&credential, "platform/backend").unwrap();

    assert_eq!(
        clone_url,
        "https://gitlab.example.com:8443/gitlab/platform/backend.git"
    );
    assert!(!clone_url.contains("secret-token"));
    assert!(!clone_url.contains('@'));
}

#[test]
fn project_api_path_is_encoded_once() {
    let credential = gitlab_credential("one", "platform/backend", "secret-token");

    let url = project_api_url(&credential, "platform/backend").unwrap();

    assert_eq!(
        url.as_str(),
        "https://gitlab.example.com:8443/gitlab/api/v4/projects/platform%2Fbackend"
    );
    assert!(!url.as_str().contains("%252F"));
}

#[test]
fn credential_ids_allow_separate_tokens_on_one_host() {
    let credentials = unique_credentials_by_id(&[
        gitlab_credential("one", "group/one", "token-one"),
        gitlab_credential("two", "group/two", "token-two"),
    ])
    .unwrap();

    assert_eq!(credentials.len(), 2);
    assert_eq!(credentials[0].host, credentials[1].host);
    assert_ne!(credentials[0].id, credentials[1].id);
}

#[test]
fn conflicting_duplicate_credential_ids_are_rejected() {
    let error = unique_credentials_by_id(&[
        gitlab_credential("same", "group/one", "token-one"),
        gitlab_credential("same", "group/two", "token-two"),
    ])
    .unwrap_err();

    assert!(error.to_string().contains("share an ID"));
}

#[test]
fn exact_path_does_not_cross_forge_boundaries() {
    let repository = SourceRepo::new(CodeForge::GitHub, "group".to_string(), "one".to_string());
    let credential = gitlab_credential("one", "group/one", "token-one");

    assert!(
        select_credential_for_repository(&[credential], &repository)
            .unwrap()
            .is_none()
    );
}

#[test]
fn exact_path_store_selects_separate_tokens_on_one_host() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("credentials");
    let one = gitlab_credential("one", "group/one", "token-one");
    let two = gitlab_credential("two", "group/two.git", "token-two");
    std::fs::write(
        &store,
        format!(
            "{}\n{}\n",
            credential_store_line(&one, "group/one").unwrap(),
            credential_store_line(&two, "group/two").unwrap()
        ),
    )
    .unwrap();

    let first = git_credential_from_store(&store, "group/one");
    let second = git_credential_from_store(&store, "group/two");

    assert!(first.contains("password=token-one"));
    assert!(!first.contains("token-two"));
    assert!(second.contains("password=token-two"));
    assert!(!second.contains("token-one"));
}

#[test]
fn glab_yaml_uses_valid_hosts_shape_and_subfolder() {
    let yaml =
        glab_config_yaml(&gitlab_credential("one", "group/one", "repository-token")).unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();

    assert_eq!(
        value["host"].as_str(),
        Some("https://gitlab.example.com:8443/gitlab")
    );
    assert_eq!(
        value["hosts"]["gitlab.example.com:8443"]["subfolder"].as_str(),
        Some("gitlab")
    );
    assert_eq!(
        value["hosts"]["gitlab.example.com:8443"]["token"].as_str(),
        Some("repository-token")
    );
    assert_eq!(
        value["hosts"]["gitlab.example.com:8443"]["git_protocol"].as_str(),
        Some("https")
    );
}

#[cfg(unix)]
#[test]
fn ordinary_glab_wrapper_discovers_each_checkout_config() {
    let temp = tempfile::tempdir().unwrap();
    let fake_glab = temp.path().join("real-glab");
    write_executable_file(
        &fake_glab,
        "#!/bin/sh\nprintf '%s\\n%s\\n%s\\n' \"$GLAB_CONFIG_DIR\" \"${GITLAB_TOKEN+set}\" \"${GITLAB_HOST+set}\"\n",
    )
    .unwrap();
    let wrapper = temp.path().join("glab");
    write_executable_file(&wrapper, &glab_wrapper_script(&fake_glab)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(temp.path().to_path_buf()).chain(
            std::env::var_os("PATH")
                .into_iter()
                .flat_map(|path| std::env::split_paths(&path)),
        ),
    )
    .unwrap();

    let mut selected_tokens = Vec::new();
    for (name, credential) in [
        ("one", gitlab_credential("one", "group/one", "token-one")),
        ("two", gitlab_credential("two", "group/two", "token-two")),
    ] {
        let repository = temp.path().join(name);
        std::fs::create_dir(&repository).unwrap();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repository)
            .status()
            .unwrap();
        assert!(init.success());
        let config_dir = repository.join(".git/warp/glab-cli");
        write_glab_config_for_credential(&credential, &config_dir).unwrap();
        run_repository_git_config(
            &repository,
            GLAB_REPOSITORY_CONFIG_KEY,
            &config_dir.to_string_lossy(),
        )
        .unwrap();

        let output = Command::new("sh")
            .args(["-c", "glab"])
            .current_dir(&repository)
            .env("PATH", &path)
            .env("GITLAB_HOST", "wrong.example.com")
            .env("GITLAB_TOKEN", "ambient-token")
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = String::from_utf8(output.stdout).unwrap();
        let mut output_lines = output.lines();
        let selected_dir = PathBuf::from(output_lines.next().unwrap());
        assert_eq!(output_lines.next(), Some(""));
        assert_eq!(output_lines.next(), Some(""));
        let config: serde_yaml::Value = serde_yaml::from_str(
            &std::fs::read_to_string(selected_dir.join("config.yml")).unwrap(),
        )
        .unwrap();
        selected_tokens.push(
            config["hosts"]["gitlab.example.com:8443"]["token"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    assert_eq!(selected_tokens, ["token-one", "token-two"]);
}

#[test]
fn refresh_merges_by_credential_id_instead_of_host() {
    let credentials = merged_credentials_by_id(
        &[
            gitlab_credential("one", "group/one", "old-one"),
            gitlab_credential("two", "group/two", "old-two"),
        ],
        &[gitlab_credential("two", "group/two", "new-two")],
    )
    .unwrap();

    assert_eq!(credentials[0].token, "old-one");
    assert_eq!(credentials[1].token, "new-two");
}

#[test]
fn diagnostics_do_not_expose_tokens_hosts_or_identity() {
    let diagnostics = credential_diagnostics(
        &[gitlab_credential(
            "credential-one",
            "group/one",
            "secret-token",
        )],
        &["gitlab.example.com".to_string()],
    );

    assert!(diagnostics.contains("credential-one"));
    assert!(diagnostics.contains("failed_credential_count=1"));
    assert!(!diagnostics.contains("secret-token"));
    assert!(!diagnostics.contains("gitlab.example.com"));
    assert!(!diagnostics.contains("oauth2"));
}

#[test]
fn bootstrap_rejects_any_partial_failure_without_naming_hosts() {
    let error = credentials_for_bootstrap(TaskGitCredentialsResponse {
        credentials: vec![github_credential()],
        failed_hosts: vec!["customer-gitlab.example.com".to_string()],
    })
    .unwrap_err();

    assert!(!error.to_string().contains("customer-gitlab.example.com"));
}

#[test]
fn git_preflight_command_has_no_tokens() {
    let repositories = [
        SourceRepo::new(CodeForge::GitLab, "group".to_string(), "one".to_string()),
        SourceRepo::new(CodeForge::GitLab, "group".to_string(), "two".to_string()),
    ];
    let repositories = repositories
        .iter()
        .map(|repository| {
            (
                repository,
                format!(
                    "https://gitlab.example.com:8443/gitlab/group/{}.git",
                    repository.repo
                ),
            )
        })
        .collect::<Vec<_>>();

    let command = git_access_check_command(&repositories, Path::new("/workspace")).unwrap();

    assert!(command.contains("/workspace/one"));
    assert!(command.contains("/workspace/two"));
    assert!(command.contains(
        "ls-remote --exit-code 'https://gitlab.example.com:8443/gitlab/group/one.git' HEAD"
    ));
    assert!(!command.contains("token-one"));
    assert!(!command.contains("token-two"));
    assert!(!command.contains("GITLAB_TOKEN"));
}

#[test]
fn managed_forge_fallback_remains_explicit_when_no_task_credential_matches() {
    let repository = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    );

    assert_eq!(
        select_credential_for_repository(&[], &repository)
            .unwrap()
            .map(|credential| credential.id.as_str()),
        None
    );
    assert_eq!(
        repository.https_clone_url(),
        "https://github.com/warpdotdev/warp.git"
    );
}
