//! Task-scoped Git credentials for cloud agent sandboxes.
//!
//! Tokens are stored only in owner-readable files. Git selects them by the
//! complete HTTPS repository path, and each GitLab checkout receives a private
//! `glab` configuration selected by a task-local wrapper.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
#[cfg(not(target_family = "wasm"))]
use async_compat::Compat;
use cloud_object_models::{CodeForge, SourceRepo};
use command::blocking::Command as BlockingCommand;
use serde::Serialize;
use url::Url;

use crate::server::server_api::ai::{AIClient, GitCredential, TaskGitCredentialsResponse};

/// Refresh before the shortest-lived one-hour token expires.
pub(crate) const GIT_CREDENTIALS_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

const DEFAULT_GIT_NAME: &str = "Warp";
const DEFAULT_GIT_EMAIL: &str = "agent@warp.dev";
const GITHUB_HOST: &str = "github.com";
const GITLAB_HOST: &str = "gitlab.com";
const GH_HOSTS_FILENAME: &str = "hosts.yml";
const GLAB_CONFIG_FILENAME: &str = "config.yml";
const TASK_CREDENTIALS_DIR: &str = ".warp/task-git";
const TASK_CREDENTIALS_FILENAME: &str = "credentials";
const TASK_BIN_DIRNAME: &str = "bin";
const GLAB_WRAPPER_FILENAME: &str = "glab";
const GLAB_REPOSITORY_CONFIG_KEY: &str = "warp.glabConfigDir";
const NETWORK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const NETWORK_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

static TASK_CREDENTIALS: RwLock<Vec<GitCredential>> = RwLock::new(Vec::new());
static REPOSITORY_BINDINGS: RwLock<HashMap<(CodeForge, String), String>> =
    RwLock::new(HashMap::new());
static GLAB_CONFIGS: RwLock<Vec<RegisteredGlabConfig>> = RwLock::new(Vec::new());

#[derive(Clone)]
struct RegisteredGlabConfig {
    credential_id: String,
    config_dir: PathBuf,
}

fn merged_credentials_by_id(
    existing: &[GitCredential],
    refreshed: &[GitCredential],
) -> Result<Vec<GitCredential>> {
    let refreshed = unique_credentials_by_id(refreshed)?;
    let mut merged = existing.to_vec();
    for credential in refreshed {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.id == credential.id)
        {
            *existing = credential;
        } else {
            merged.push(credential);
        }
    }
    Ok(merged)
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitLabPreflightError {
    #[error("The worker could not reach the GitLab instance.")]
    Connectivity,
    #[error("The worker could not establish trusted TLS with the GitLab instance.")]
    Tls,
    #[error("The worker GitLab credential was rejected.")]
    Authentication,
    #[error("The worker GitLab credential cannot access a required repository.")]
    RepositoryAccess,
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

fn task_credentials_root(home: &Path) -> PathBuf {
    home.join(TASK_CREDENTIALS_DIR)
}

fn task_credentials_file(home: &Path) -> PathBuf {
    task_credentials_root(home).join(TASK_CREDENTIALS_FILENAME)
}

fn task_bin_dir_for_home(home: &Path) -> PathBuf {
    task_credentials_root(home).join(TASK_BIN_DIRNAME)
}

pub(crate) fn task_bin_dir() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    let bin_dir = task_bin_dir_for_home(&home);
    bin_dir
        .join(GLAB_WRAPPER_FILENAME)
        .is_file()
        .then_some(bin_dir)
}

/// Write secret content with owner-only permissions.
fn write_secret_file(path: &Path, content: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {} for writing", path.display()))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn write_executable_file(path: &Path, content: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    write_secret_file(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Failed to make {} executable", path.display()))
}

fn normalized_project_path(path: &str) -> Result<String> {
    let path = path.trim().trim_matches('/');
    let path = if path
        .get(path.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".git"))
    {
        &path[..path.len() - 4]
    } else {
        path
    };
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("Invalid repository credential path");
    }
    Ok(path.to_string())
}

fn project_key(path: &str) -> Result<String> {
    normalized_project_path(path)
}

fn source_repo_project_path(repo: &SourceRepo) -> String {
    format!("{}/{}", repo.owner, repo.repo)
}

fn repository_binding_key(repo: &SourceRepo) -> Result<(CodeForge, String)> {
    let code_forge = repo
        .code_forge
        .filter(|code_forge| !matches!(code_forge, CodeForge::None | CodeForge::Unknown))
        .ok_or_else(|| anyhow::anyhow!("Repository has no supported code forge"))?;
    Ok((code_forge, project_key(&source_repo_project_path(repo))?))
}

fn normalized_relative_prefix(prefix: &str) -> Result<String> {
    let prefix = prefix.trim().trim_matches('/');
    if prefix.contains('\\')
        || (!prefix.is_empty()
            && prefix
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | "..")))
    {
        bail!("Invalid GitLab relative URL prefix");
    }
    Ok(prefix.to_string())
}

fn credential_origin(credential: &GitCredential) -> Result<Url> {
    if credential.scheme != "https" {
        bail!("Task Git credentials require HTTPS");
    }
    if credential.host.trim().is_empty()
        || credential.host.trim() != credential.host
        || credential.host.contains('/')
        || credential.host.contains('@')
    {
        bail!("Invalid task Git credential host");
    }
    let mut origin = Url::parse("https://invalid/")
        .context("Failed to initialize task Git credential origin")?;
    origin
        .set_host(Some(&credential.host))
        .map_err(|_| anyhow::anyhow!("Invalid task Git credential host"))?;
    if let Some(port) = credential.port {
        let port = u16::try_from(port)
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| anyhow::anyhow!("Invalid task Git credential port"))?;
        origin
            .set_port(Some(port))
            .map_err(|_| anyhow::anyhow!("Invalid task Git credential port"))?;
    }
    let prefix = normalized_relative_prefix(&credential.relative_url_prefix)?;
    origin.set_path(if prefix.is_empty() {
        "/"
    } else {
        &format!("/{prefix}/")
    });
    Ok(origin)
}

fn credential_authority(credential: &GitCredential) -> Result<String> {
    let origin = credential_origin(credential)?;
    Ok(match origin.port() {
        Some(port) => format!(
            "{}:{port}",
            origin
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("Task Git credential origin has no host"))?
        ),
        None => origin
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Task Git credential origin has no host"))?
            .to_string(),
    })
}

fn clone_url_for_path(credential: &GitCredential, project_path: &str) -> Result<String> {
    let project_path = normalized_project_path(project_path)?;
    let prefix = normalized_relative_prefix(&credential.relative_url_prefix)?;
    let mut clone_url = credential_origin(credential)?;
    let path = if prefix.is_empty() {
        format!("/{project_path}.git")
    } else {
        format!("/{prefix}/{project_path}.git")
    };
    clone_url.set_path(&path);
    Ok(clone_url.to_string())
}

fn is_gitlab_credential(credential: &GitCredential) -> bool {
    credential.instance_uid.is_some()
        || credential.installation_uid.is_some()
        || credential.host.eq_ignore_ascii_case(GITLAB_HOST)
}

fn is_self_hosted_gitlab_credential(credential: &GitCredential) -> bool {
    is_gitlab_credential(credential)
        && (!credential.host.eq_ignore_ascii_case(GITLAB_HOST)
            || credential.port.is_some()
            || !credential.relative_url_prefix.trim_matches('/').is_empty())
}

fn credential_authorizes_path(credential: &GitCredential, path_key: &str) -> bool {
    credential.project_paths.iter().any(|path| {
        project_key(path).is_ok_and(|credential_path_key| credential_path_key == path_key)
    })
}
fn credential_matches_forge(credential: &GitCredential, code_forge: Option<CodeForge>) -> bool {
    match code_forge {
        Some(CodeForge::GitHub) => {
            credential.host.eq_ignore_ascii_case(GITHUB_HOST) && !is_gitlab_credential(credential)
        }
        Some(CodeForge::GitLab) => is_gitlab_credential(credential),
        Some(CodeForge::None | CodeForge::Unknown) | None => false,
    }
}

fn credentials_equal(left: &GitCredential, right: &GitCredential) -> bool {
    left.id == right.id
        && left.instance_uid == right.instance_uid
        && left.installation_uid == right.installation_uid
        && left.scheme == right.scheme
        && left.host == right.host
        && left.port == right.port
        && left.relative_url_prefix == right.relative_url_prefix
        && left.project_paths == right.project_paths
        && left.token == right.token
        && left.username == right.username
        && left.email == right.email
}

/// Keep one credential per opaque ID. Different credentials may share a host.
fn unique_credentials_by_id(credentials: &[GitCredential]) -> Result<Vec<GitCredential>> {
    let mut index_by_id = HashMap::new();
    let mut unique = Vec::new();
    for credential in credentials {
        if credential.id.trim().is_empty() {
            bail!("Task Git credential is missing an ID");
        }
        credential_origin(credential)?;
        for project_path in &credential.project_paths {
            normalized_project_path(project_path)?;
        }

        if let Some(&index) = index_by_id.get(&credential.id) {
            let existing: &GitCredential = &unique[index];
            if !credentials_equal(existing, credential) {
                bail!("Conflicting task Git credentials share an ID");
            }
            continue;
        }
        index_by_id.insert(credential.id.clone(), unique.len());
        unique.push(credential.clone());
    }
    Ok(unique)
}

fn task_credentials_snapshot() -> Result<Vec<GitCredential>> {
    TASK_CREDENTIALS
        .read()
        .map(|credentials| credentials.clone())
        .map_err(|_| anyhow::anyhow!("Task Git credential state is unavailable"))
}

fn replace_task_credentials(credentials: &[GitCredential]) -> Result<Vec<GitCredential>> {
    let credentials = unique_credentials_by_id(credentials)?;
    let mut stored = TASK_CREDENTIALS
        .write()
        .map_err(|_| anyhow::anyhow!("Task Git credential state is unavailable"))?;
    *stored = credentials.clone();
    Ok(credentials)
}

fn merge_task_credentials(credentials: &[GitCredential]) -> Result<Vec<GitCredential>> {
    let mut stored = TASK_CREDENTIALS
        .write()
        .map_err(|_| anyhow::anyhow!("Task Git credential state is unavailable"))?;
    *stored = merged_credentials_by_id(&stored, credentials)?;
    Ok(stored.clone())
}

fn select_credential_for_repository<'a>(
    credentials: &'a [GitCredential],
    repo: &SourceRepo,
) -> Result<Option<&'a GitCredential>> {
    let path_key = project_key(&source_repo_project_path(repo))?;
    let exact = credentials
        .iter()
        .filter(|credential| {
            credential_matches_forge(credential, repo.code_forge)
                && credential_authorizes_path(credential, &path_key)
        })
        .collect::<Vec<_>>();
    match exact.as_slice() {
        [credential] => return Ok(Some(*credential)),
        [] => {}
        _ => bail!("Multiple task Git credentials authorize the same repository path"),
    }

    let expected_host = repo.code_forge.map(CodeForge::host).unwrap_or_default();
    let fallback = credentials
        .iter()
        .filter(|credential| {
            credential_matches_forge(credential, repo.code_forge)
                && credential.project_paths.is_empty()
                && credential.host.eq_ignore_ascii_case(expected_host)
        })
        .collect::<Vec<_>>();
    match fallback.as_slice() {
        [credential] => Ok(Some(*credential)),
        [] => Ok(None),
        _ => bail!("Multiple host-wide task Git credentials match one repository"),
    }
}

fn binding_credential_for_repository(
    credentials: &[GitCredential],
    repo: &SourceRepo,
) -> Result<Option<GitCredential>> {
    let key = repository_binding_key(repo)?;
    if let Some(credential_id) = REPOSITORY_BINDINGS
        .read()
        .map_err(|_| anyhow::anyhow!("Task repository credential state is unavailable"))?
        .get(&key)
        .cloned()
    {
        return credentials
            .iter()
            .find(|credential| credential.id == credential_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("Task repository credential is unavailable"));
    }
    Ok(select_credential_for_repository(credentials, repo)?.cloned())
}

/// Register exact credential bindings before cloning and expand legacy
/// host-wide credentials into the repositories required by this task.
pub(crate) fn prepare_repository_credentials(repositories: &[SourceRepo]) -> Result<()> {
    let credentials = task_credentials_snapshot()?;
    let mut bindings = REPOSITORY_BINDINGS
        .write()
        .map_err(|_| anyhow::anyhow!("Task repository credential state is unavailable"))?;
    bindings.clear();

    let has_self_hosted_gitlab = credentials.iter().any(is_self_hosted_gitlab_credential);
    for repository in repositories {
        let binding_key = repository_binding_key(repository)?;
        match select_credential_for_repository(&credentials, repository)? {
            Some(credential) => {
                bindings.insert(binding_key, credential.id.clone());
            }
            None if repository.code_forge == Some(CodeForge::GitLab) && has_self_hosted_gitlab => {
                bail!("A self-hosted GitLab repository has no task credential binding");
            }
            None => {}
        }
    }
    drop(bindings);

    write_task_credentials_file(&credentials, &home_dir()?)
}

/// Register one repository needed before environment metadata is available,
/// such as a repository-qualified task skill.
pub(crate) fn prepare_project_path(
    code_forge: CodeForge,
    owner: &str,
    repository: &str,
) -> Result<()> {
    let source = SourceRepo::new(code_forge, owner.to_string(), repository.to_string());
    let credentials = task_credentials_snapshot()?;
    if let Some(credential) = select_credential_for_repository(&credentials, &source)? {
        REPOSITORY_BINDINGS
            .write()
            .map_err(|_| anyhow::anyhow!("Task repository credential state is unavailable"))?
            .insert(repository_binding_key(&source)?, credential.id.clone());
        write_task_credentials_file(&credentials, &home_dir()?)?;
    }
    Ok(())
}

/// Resolve an explicit, credential-owned clone origin. Public repositories
/// without task credentials retain the managed forge URL.
pub(crate) fn clone_url_for_repository(repo: &SourceRepo) -> Result<String> {
    let credentials = task_credentials_snapshot()?;
    match binding_credential_for_repository(&credentials, repo)? {
        Some(credential) => clone_url_for_path(&credential, &source_repo_project_path(repo)),
        None => Ok(repo.https_clone_url()),
    }
}

fn paths_for_credential(credential: &GitCredential) -> Result<BTreeSet<String>> {
    let mut paths = credential
        .project_paths
        .iter()
        .map(|path| normalized_project_path(path))
        .collect::<Result<BTreeSet<_>>>()?;
    let bindings = REPOSITORY_BINDINGS
        .read()
        .map_err(|_| anyhow::anyhow!("Task repository credential state is unavailable"))?;
    paths.extend(
        bindings
            .iter()
            .filter(|(_, credential_id)| *credential_id == &credential.id)
            .map(|((_, path), _)| path.clone()),
    );
    Ok(paths)
}

fn credential_store_line(credential: &GitCredential, project_path: &str) -> Result<String> {
    let mut url = Url::parse(&clone_url_for_path(credential, project_path)?)
        .context("Invalid task Git credential repository URL")?;
    let username = credential.username.as_deref().unwrap_or_else(|| {
        if is_gitlab_credential(credential) {
            "oauth2"
        } else {
            "x-access-token"
        }
    });
    url.set_username(username)
        .map_err(|_| anyhow::anyhow!("Invalid task Git credential username"))?;
    url.set_password(Some(&credential.token))
        .map_err(|_| anyhow::anyhow!("Invalid task Git credential token"))?;
    Ok(url.to_string())
}

fn write_task_credentials_file(credentials: &[GitCredential], home: &Path) -> Result<()> {
    let root = task_credentials_root(home);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("Failed to create {}", root.display()))?;
    let path = task_credentials_file(home);
    let tmp_path = path.with_extension("tmp");

    let mut lines = Vec::new();
    for credential in credentials {
        for project_path in paths_for_credential(credential)? {
            lines.push(credential_store_line(credential, &project_path)?);
        }
    }
    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    write_secret_file(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("Failed to install {}", path.display()))
}

fn write_gh_hosts_yml(credentials: &[GitCredential], home: &Path) -> Result<()> {
    let github_credentials = credentials
        .iter()
        .filter(|credential| credential.host.eq_ignore_ascii_case(GITHUB_HOST))
        .collect::<Vec<_>>();
    if github_credentials.is_empty() {
        return Ok(());
    }
    if github_credentials.len() > 1 {
        bail!("Multiple GitHub task credentials cannot share the gh host configuration");
    }

    let gh_config_dir = home.join(".config").join("gh");
    std::fs::create_dir_all(&gh_config_dir)
        .with_context(|| format!("Failed to create {}", gh_config_dir.display()))?;
    let path = gh_config_dir.join(GH_HOSTS_FILENAME);
    let tmp_path = gh_config_dir.join(format!("{GH_HOSTS_FILENAME}.tmp"));
    let credential = github_credentials[0];
    let mut yaml = format!(
        "{}:\n    oauth_token: {}\n    git_protocol: https\n",
        credential.host, credential.token
    );
    if let Some(username) = &credential.username {
        yaml.push_str(&format!("    user: {username}\n"));
    }

    write_secret_file(&tmp_path, &yaml)?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("Failed to install {}", path.display()))
}

#[derive(Serialize)]
struct GlabConfig {
    git_protocol: &'static str,
    host: String,
    no_prompt: bool,
    telemetry: bool,
    hosts: BTreeMap<String, GlabHostConfig>,
}

#[derive(Serialize)]
struct GlabHostConfig {
    api_protocol: String,
    api_host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subfolder: Option<String>,
    token: String,
    git_protocol: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

fn glab_config_yaml(credential: &GitCredential) -> Result<String> {
    let authority = credential_authority(credential)?;
    let prefix = normalized_relative_prefix(&credential.relative_url_prefix)?;
    let mut hosts = BTreeMap::new();
    hosts.insert(
        authority.clone(),
        GlabHostConfig {
            api_protocol: credential.scheme.clone(),
            api_host: authority,
            subfolder: (!prefix.is_empty()).then_some(prefix),
            token: credential.token.clone(),
            git_protocol: "https",
            user: credential.username.clone(),
        },
    );
    let origin = credential_origin(credential)?;
    serde_yaml::to_string(&GlabConfig {
        git_protocol: "https",
        host: origin.as_str().trim_end_matches('/').to_string(),
        no_prompt: true,
        telemetry: false,
        hosts,
    })
    .context("Failed to serialize repository-local glab configuration")
}

fn write_glab_config_for_credential(credential: &GitCredential, config_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("Failed to create {}", config_dir.display()))?;
    let path = config_dir.join(GLAB_CONFIG_FILENAME);
    let tmp_path = config_dir.join(format!("{GLAB_CONFIG_FILENAME}.tmp"));
    write_secret_file(&tmp_path, &glab_config_yaml(credential)?)?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("Failed to install {}", path.display()))
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\"'\"'")
}

#[cfg(unix)]
fn find_real_glab(wrapper_path: &Path) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(GLAB_WRAPPER_FILENAME))
        .find(|candidate| candidate.is_file() && candidate != wrapper_path)
}

#[cfg(unix)]
fn glab_wrapper_script(real_glab: &Path) -> String {
    let real_glab = shell_single_quote(&real_glab.to_string_lossy());
    format!(
        "#!/bin/sh\n\
         unset GITLAB_HOST GITLAB_TOKEN\n\
         repo_root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exec '{real_glab}' \"$@\"\n\
         config_dir=\"$(git -C \"$repo_root\" config --local --path --get {GLAB_REPOSITORY_CONFIG_KEY} 2>/dev/null)\"\n\
         if [ -n \"$config_dir\" ]; then\n\
           GLAB_CONFIG_DIR=\"$config_dir\" exec '{real_glab}' \"$@\"\n\
         fi\n\
         exec '{real_glab}' \"$@\"\n"
    )
}

fn install_glab_wrapper(credentials: &[GitCredential], home: &Path) -> Result<()> {
    if !credentials.iter().any(is_gitlab_credential) {
        return Ok(());
    }

    #[cfg(unix)]
    {
        let bin_dir = task_bin_dir_for_home(home);
        std::fs::create_dir_all(&bin_dir)
            .with_context(|| format!("Failed to create {}", bin_dir.display()))?;
        let wrapper_path = bin_dir.join(GLAB_WRAPPER_FILENAME);
        let real_glab = find_real_glab(&wrapper_path)
            .ok_or_else(|| anyhow::anyhow!("The glab executable is required for GitLab tasks"))?;
        write_executable_file(&wrapper_path, &glab_wrapper_script(&real_glab))?;
    }
    Ok(())
}

fn repository_glab_config_dir(repository_dir: &Path) -> Result<PathBuf> {
    let directory = repository_dir.to_string_lossy();
    let output = BlockingCommand::new("git")
        .args([
            "-C",
            directory.as_ref(),
            "rev-parse",
            "--git-path",
            "warp/glab-cli",
        ])
        .output()
        .context("Failed to locate the repository Git directory")?;
    if !output.status.success() {
        bail!("Failed to locate the repository Git directory");
    }
    let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repository_dir.join(path)
    })
}

fn run_repository_git_config(repository_dir: &Path, key: &str, value: &str) -> Result<()> {
    let directory = repository_dir.to_string_lossy();
    let output = BlockingCommand::new("git")
        .args(["-C", directory.as_ref(), "config", key, value])
        .output()
        .context("Failed to configure a task repository")?;
    if !output.status.success() {
        bail!("Failed to configure a task repository");
    }
    Ok(())
}

fn register_glab_config(credential_id: &str, config_dir: &Path) -> Result<()> {
    let mut configs = GLAB_CONFIGS
        .write()
        .map_err(|_| anyhow::anyhow!("Repository-local glab state is unavailable"))?;
    if let Some(config) = configs
        .iter_mut()
        .find(|config| config.config_dir == config_dir)
    {
        config.credential_id = credential_id.to_string();
    } else {
        configs.push(RegisteredGlabConfig {
            credential_id: credential_id.to_string(),
            config_dir: config_dir.to_path_buf(),
        });
    }
    Ok(())
}

fn write_registered_glab_configs(credentials: &[GitCredential]) -> Result<()> {
    let configs = GLAB_CONFIGS
        .read()
        .map_err(|_| anyhow::anyhow!("Repository-local glab state is unavailable"))?
        .clone();
    for config in configs {
        let credential = credentials
            .iter()
            .find(|credential| credential.id == config.credential_id)
            .ok_or_else(|| anyhow::anyhow!("Repository-local glab credential is unavailable"))?;
        write_glab_config_for_credential(credential, &config.config_dir)?;
    }
    Ok(())
}

fn identity_of(credential: Option<&GitCredential>) -> (String, String) {
    match credential {
        Some(credential) => (
            credential
                .username
                .as_deref()
                .unwrap_or(DEFAULT_GIT_NAME)
                .to_string(),
            credential
                .email
                .as_deref()
                .unwrap_or(DEFAULT_GIT_EMAIL)
                .to_string(),
        ),
        None => (DEFAULT_GIT_NAME.to_string(), DEFAULT_GIT_EMAIL.to_string()),
    }
}

/// Install identity and repository-local `glab` configuration after cloning.
pub(crate) fn configure_repository_credentials(
    repository_dir: &Path,
    repository: &SourceRepo,
) -> Result<()> {
    let credentials = task_credentials_snapshot()?;
    let credential = binding_credential_for_repository(&credentials, repository)?;
    let (name, email) = identity_of(credential.as_ref());
    run_repository_git_config(repository_dir, "user.name", &name)?;
    run_repository_git_config(repository_dir, "user.email", &email)?;

    let Some(credential) = credential.filter(is_gitlab_credential) else {
        return Ok(());
    };
    let config_dir = repository_glab_config_dir(repository_dir)?;
    write_glab_config_for_credential(&credential, &config_dir)?;
    run_repository_git_config(
        repository_dir,
        GLAB_REPOSITORY_CONFIG_KEY,
        &config_dir.to_string_lossy(),
    )?;
    register_glab_config(&credential.id, &config_dir)
}

fn run_git_config(args: &[&str]) -> Result<()> {
    let output = BlockingCommand::new("git")
        .arg("config")
        .arg("--global")
        .args(args)
        .output()
        .context("Failed to run global Git configuration")?;
    if !output.status.success() {
        bail!("Failed to run global Git configuration");
    }
    Ok(())
}

fn clear_git_credential_helpers() -> Result<()> {
    let output = BlockingCommand::new("git")
        .args(["config", "--global", "--unset-all", "credential.helper"])
        .output()
        .context("Failed to clear global Git credential helpers")?;
    if output.status.success() || output.status.code() == Some(5) {
        Ok(())
    } else {
        bail!("Failed to clear global Git credential helpers")
    }
}

fn setup_git_config(credentials: &[GitCredential], home: &Path) -> Result<()> {
    clear_git_credential_helpers()?;
    let helper = format!(
        "store --file={}",
        task_credentials_file(home).to_string_lossy()
    );
    run_git_config(&["credential.helper", &helper])?;
    run_git_config(&["credential.useHttpPath", "true"])?;

    let mut configured_origins = HashSet::new();
    for credential in credentials {
        let origin = credential_origin(credential)?;
        let rewrite_base = origin.as_str().to_string();
        if !configured_origins.insert(rewrite_base.clone()) {
            continue;
        }
        let key = format!("url.{rewrite_base}.insteadOf");
        let ssh_url = format!("ssh://git@{}/", credential.host);
        run_git_config(&["--add", &key, &ssh_url])?;
        let scp_url = format!("git@{}:", credential.host);
        run_git_config(&["--add", &key, &scp_url])?;
    }
    Ok(())
}

fn configure_global_git_identity(credentials: &[GitCredential]) -> Result<()> {
    let (name, email) = identity_of(credentials.first());
    run_git_config(&["user.name", &name])?;
    run_git_config(&["user.email", &email])
}

/// Bootstrap has no previous store, so partial failure cannot safely proceed.
pub(crate) fn credentials_for_bootstrap(
    response: TaskGitCredentialsResponse,
) -> Result<Vec<GitCredential>> {
    if !response.failed_hosts.is_empty() {
        bail!(
            "Git credential bootstrap cannot proceed because one or more credentials failed to refresh"
        );
    }
    unique_credentials_by_id(&response.credentials)
}

/// Formats non-sensitive metadata for local credential diagnostics.
pub(crate) fn credential_diagnostics(
    credentials: &[GitCredential],
    failed_hosts: &[String],
) -> String {
    let refreshed = credentials
        .iter()
        .map(|credential| {
            format!(
                "{}(refreshed, token_present={}, username_present={})",
                credential.id,
                !credential.token.is_empty(),
                credential.username.is_some()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{refreshed}; failed_credential_count={}",
        failed_hosts.len()
    )
}

pub(crate) fn configure_git_credentials(credentials: &[GitCredential]) -> Result<()> {
    let credentials = replace_task_credentials(credentials)?;
    REPOSITORY_BINDINGS
        .write()
        .map_err(|_| anyhow::anyhow!("Task repository credential state is unavailable"))?
        .clear();
    GLAB_CONFIGS
        .write()
        .map_err(|_| anyhow::anyhow!("Repository-local glab state is unavailable"))?
        .clear();
    if credentials.is_empty() {
        return Ok(());
    }
    let home = home_dir()?;
    std::fs::create_dir_all(task_credentials_root(&home)).with_context(|| {
        format!(
            "Failed to create {}",
            task_credentials_root(&home).display()
        )
    })?;
    install_glab_wrapper(&credentials, &home)?;
    setup_git_config(&credentials, &home)?;
    configure_global_git_identity(&credentials)?;
    write_task_credentials_file(&credentials, &home)?;
    write_gh_hosts_yml(&credentials, &home)?;
    log::info!(
        "Configured {} task Git credential(s): {}",
        credentials.len(),
        credential_diagnostics(&credentials, &[])
    );
    Ok(())
}

fn apply_refreshed_credentials(response: TaskGitCredentialsResponse) -> Result<bool> {
    if response.credentials.is_empty() && response.failed_hosts.is_empty() {
        log::debug!("No Git credentials returned during refresh; skipping file write");
        return Ok(true);
    }

    let credentials = merge_task_credentials(&response.credentials)?;
    let home = home_dir()?;
    write_task_credentials_file(&credentials, &home)
        .context("Failed to write refreshed task Git credentials")?;
    write_gh_hosts_yml(&credentials, &home)
        .context("Failed to write refreshed GitHub CLI credentials")?;
    write_registered_glab_configs(&credentials)
        .context("Failed to write refreshed repository-local glab credentials")?;
    log::info!(
        "Refreshed task Git credentials: {}",
        credential_diagnostics(&response.credentials, &response.failed_hosts)
    );
    Ok(response.failed_hosts.is_empty())
}

fn self_hosted_gitlab_credentials_for_repositories(
    repositories: &[SourceRepo],
) -> Result<Vec<(GitCredential, Vec<String>)>> {
    let credentials = task_credentials_snapshot()?;
    let mut by_id = BTreeMap::<String, (GitCredential, Vec<String>)>::new();
    for repository in repositories {
        let Some(credential) = binding_credential_for_repository(&credentials, repository)? else {
            continue;
        };
        if !is_self_hosted_gitlab_credential(&credential) {
            continue;
        }
        let project_path = normalized_project_path(&source_repo_project_path(repository))?;
        by_id
            .entry(credential.id.clone())
            .or_insert_with(|| (credential, Vec::new()))
            .1
            .push(project_path);
    }
    Ok(by_id.into_values().collect())
}

fn transport_error_is_tls(error: &reqwest::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    [
        "certificate",
        "tls",
        "ssl",
        "unknown issuer",
        "invalid peer",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(not(target_family = "wasm"))]
async fn check_tcp_connectivity(credential: &GitCredential) -> Result<(), GitLabPreflightError> {
    let port = credential
        .port
        .map(u16::try_from)
        .transpose()
        .map_err(|_| GitLabPreflightError::Connectivity)?
        .unwrap_or(443);
    let host = credential.host.clone();
    tokio::time::timeout(NETWORK_CONNECT_TIMEOUT, async move {
        let addresses = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| GitLabPreflightError::Connectivity)?;
        for address in addresses {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                return Ok(());
            }
        }
        Err(GitLabPreflightError::Connectivity)
    })
    .await
    .map_err(|_| GitLabPreflightError::Connectivity)?
}

fn endpoint_url(credential: &GitCredential, suffix: &str) -> Result<Url> {
    let prefix = normalized_relative_prefix(&credential.relative_url_prefix)?;
    let mut url = credential_origin(credential)?;
    let path = if prefix.is_empty() {
        format!("/{suffix}")
    } else {
        format!("/{prefix}/{suffix}")
    };
    url.set_path(&path);
    Ok(url)
}

fn project_api_url(credential: &GitCredential, project_path: &str) -> Result<Url> {
    let mut url = endpoint_url(credential, "api/v4/projects")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid GitLab project API URL"))?
        .push(project_path);
    Ok(url)
}

#[cfg(not(target_family = "wasm"))]
async fn preflight_gitlab_credential(
    credential: &GitCredential,
    project_paths: &[String],
) -> Result<(), GitLabPreflightError> {
    Compat::new(async {
        check_tcp_connectivity(credential).await?;

        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(NETWORK_CONNECT_TIMEOUT)
            .timeout(NETWORK_REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| GitLabPreflightError::Tls)?;

        let tls_response = client
            .get(
                endpoint_url(credential, "-/readiness")
                    .map_err(|_| GitLabPreflightError::Connectivity)?,
            )
            .send()
            .await;
        if let Err(error) = tls_response {
            return Err(if transport_error_is_tls(&error) {
                GitLabPreflightError::Tls
            } else {
                GitLabPreflightError::Connectivity
            });
        }

        let api_response = client
            .get(
                endpoint_url(credential, "api/v4/user")
                    .map_err(|_| GitLabPreflightError::Authentication)?,
            )
            .bearer_auth(&credential.token)
            .send()
            .await
            .map_err(|error| {
                if transport_error_is_tls(&error) {
                    GitLabPreflightError::Tls
                } else {
                    GitLabPreflightError::Connectivity
                }
            })?;
        if !api_response.status().is_success() {
            return Err(GitLabPreflightError::Authentication);
        }

        for project_path in project_paths {
            let repository_response = client
                .get(
                    project_api_url(credential, project_path)
                        .map_err(|_| GitLabPreflightError::RepositoryAccess)?,
                )
                .bearer_auth(&credential.token)
                .send()
                .await
                .map_err(|error| {
                    if transport_error_is_tls(&error) {
                        GitLabPreflightError::Tls
                    } else {
                        GitLabPreflightError::Connectivity
                    }
                })?;
            if repository_response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(GitLabPreflightError::Authentication);
            }
            if !repository_response.status().is_success() {
                return Err(GitLabPreflightError::RepositoryAccess);
            }
        }
        Ok(())
    })
    .await
}

/// Run bounded DNS/TCP, TLS, API-authentication, and repository checks.
pub(crate) async fn preflight_gitlab_network(
    repositories: &[SourceRepo],
) -> Result<(), GitLabPreflightError> {
    #[cfg(not(target_family = "wasm"))]
    {
        let targets = self_hosted_gitlab_credentials_for_repositories(repositories)
            .map_err(|_| GitLabPreflightError::RepositoryAccess)?;
        for (credential, project_paths) in targets {
            preflight_gitlab_credential(&credential, &project_paths).await?;
        }
    }
    Ok(())
}

/// Build a token-free Git remote check for every self-hosted GitLab checkout.
pub(crate) fn gitlab_git_access_check_command(
    repositories: &[SourceRepo],
    working_dir: &Path,
) -> Result<Option<String>> {
    let credentials = task_credentials_snapshot()?;
    let mut repositories_to_check = Vec::new();
    for repository in repositories {
        let Some(credential) = binding_credential_for_repository(&credentials, repository)? else {
            continue;
        };
        if is_self_hosted_gitlab_credential(&credential) {
            repositories_to_check.push((
                repository,
                clone_url_for_path(&credential, &source_repo_project_path(repository))?,
            ));
        }
    }
    Ok(git_access_check_command(
        &repositories_to_check,
        working_dir,
    ))
}

fn git_access_check_command(
    repositories: &[(&SourceRepo, String)],
    working_dir: &Path,
) -> Option<String> {
    let mut script = String::new();
    for (repository, clone_url) in repositories {
        let repository_dir = working_dir.join(&repository.repo);
        let repository_dir = shell_single_quote(&repository_dir.to_string_lossy());
        let clone_url = shell_single_quote(clone_url);
        script.push_str(&format!(
            "git -C '{repository_dir}' ls-remote --exit-code '{clone_url}' HEAD >/dev/null 2>&1 || exit 1\n"
        ));
    }
    if script.is_empty() {
        return None;
    }
    Some(format!("sh -c '{}'", shell_single_quote(&script)))
}

#[tracing::instrument(name = "git_credentials::try_refresh", skip_all, err, fields(
    tags.cloud_agent = true,
    task_id,
))]
async fn try_refresh(task_id: &str, ai_client: &Arc<dyn AIClient>) -> Result<bool> {
    let workload_token =
        warp_isolation_platform::issue_workload_token(Some(Duration::from_secs(5 * 60)))
            .await
            .context("Failed to issue workload token for Git credentials refresh")?
            .token;

    let response = ai_client
        .get_task_git_credentials(task_id.to_string(), workload_token, true)
        .await
        .context("Failed to fetch Git credentials from server")?;

    apply_refreshed_credentials(response)
}

/// Refresh task credentials until the harness finishes and drops this future.
pub(crate) async fn refresh_loop(task_id: String, ai_client: Arc<dyn AIClient>) {
    loop {
        warpui::r#async::Timer::after(GIT_CREDENTIALS_REFRESH_INTERVAL).await;

        log::info!("Refreshing Git credentials for task {task_id}");

        let backoff_delays = [
            Duration::from_secs(60),
            Duration::from_secs(2 * 60),
            Duration::from_secs(4 * 60),
        ];
        let mut attempt = 0usize;
        loop {
            match try_refresh(&task_id, &ai_client).await {
                Ok(true) => break,
                Ok(false) if attempt < backoff_delays.len() => {
                    let delay = backoff_delays[attempt];
                    log::warn!(
                        "Git credentials refreshed partially (attempt {}); retrying in {}s",
                        attempt + 1,
                        delay.as_secs()
                    );
                    warpui::r#async::Timer::after(delay).await;
                    attempt += 1;
                }
                Ok(false) => {
                    log::warn!(
                        "Some Git credentials remain stale after {} attempts",
                        attempt + 1
                    );
                    break;
                }
                Err(error) if attempt < backoff_delays.len() => {
                    let delay = backoff_delays[attempt];
                    log::warn!(
                        "Git credentials refresh failed (attempt {}): {error:#}; retrying in {}s",
                        attempt + 1,
                        delay.as_secs()
                    );
                    warpui::r#async::Timer::after(delay).await;
                    attempt += 1;
                }
                Err(error) => {
                    log::warn!(
                        "Git credentials refresh failed after {} attempts: {error:#}",
                        attempt + 1
                    );
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "git_credentials_tests.rs"]
mod tests;
