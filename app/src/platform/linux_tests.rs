use super::*;

#[test]
fn parses_app_id_and_app_path_from_a_real_flatpak_info_file() {
    let contents = r#"[Application]
name=dev.warp.Warp
runtime=org.freedesktop.Platform/x86_64/24.08

[Instance]
instance-id=abc123
instance-path=/home/user/.var/app/dev.warp.Warp
app-path=/var/lib/flatpak/app/dev.warp.Warp/x86_64/stable/active/files
arch=x86_64
branch=stable
flatpak-version=1.14.6
"#;

    assert_eq!(
        parse_flatpak_info(contents),
        Some(FlatpakInfo {
            app_id: "dev.warp.Warp".to_owned(),
            app_path: "/var/lib/flatpak/app/dev.warp.Warp/x86_64/stable/active/files".to_owned(),
        })
    );
}

#[test]
fn returns_none_outside_a_flatpak_sandbox() {
    assert_eq!(parse_flatpak_info(""), None);
}

#[test]
fn returns_none_when_only_one_of_the_two_fields_is_present() {
    let contents = "[Application]\nname=dev.warp.Warp\n";
    assert_eq!(parse_flatpak_info(contents), None);
}
