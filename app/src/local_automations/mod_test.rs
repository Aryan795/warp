use std::path::PathBuf;

use super::*;

fn agent_automation() -> LocalAutomation {
    LocalAutomation::parse(
        r#"
name = "Morning repo brief"
schedule = "0 9 * * 1-5"
cwd = "~/code/warp"

[runner]
type = "warp_agent"
prompt = "Summarize commits on main from the last 24h."
"#,
    )
    .unwrap()
}

#[test]
fn promote_prompt_with_source_path_points_at_the_toml() {
    let mut automation = agent_automation();
    automation.source_path = Some(PathBuf::from("/tmp/automations/morning_brief.toml"));

    let prompt = promote_automation_agent_prompt(&automation);
    assert!(prompt.contains("create-automation skill"), "{prompt}");
    assert!(prompt.contains("\"Morning repo brief\""), "{prompt}");
    assert!(prompt.contains("morning_brief.toml"), "{prompt}");
    assert!(prompt.contains("Oz cloud schedule"), "{prompt}");
    assert!(prompt.contains("Oz environment"), "{prompt}");
}

#[test]
fn promote_prompt_without_source_path_embeds_key_fields() {
    let automation = agent_automation();
    assert_eq!(automation.source_path, None);

    let prompt = promote_automation_agent_prompt(&automation);
    assert!(prompt.contains("\"Morning repo brief\""), "{prompt}");
    assert!(prompt.contains("0 9 * * 1-5"), "{prompt}");
    assert!(
        prompt.contains("Summarize commits on main from the last 24h."),
        "{prompt}"
    );
}

#[test]
fn promote_prompt_without_source_path_embeds_shell_command() {
    let automation = LocalAutomation::parse(
        r#"
name = "PR sweep"
schedule = "@daily"
cwd = "/tmp"

[runner]
type = "shell"
command = "gh pr list --author @me"
"#,
    )
    .unwrap();

    let prompt = promote_automation_external_prompt(&automation);
    assert!(prompt.contains("Warp local automation"), "{prompt}");
    assert!(prompt.contains("gh pr list --author @me"), "{prompt}");
}

#[test]
fn external_promote_prompt_names_warp() {
    let mut automation = agent_automation();
    automation.source_path = Some(PathBuf::from("/tmp/automations/morning_brief.toml"));

    let prompt = promote_automation_external_prompt(&automation);
    assert!(prompt.contains("Warp local automation"), "{prompt}");
    assert!(prompt.contains("morning_brief.toml"), "{prompt}");
}
