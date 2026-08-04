use super::*;

#[derive(Debug, Clone)]
struct TestAction;

/// The copy button copies `to_plain_text()`, so it must concatenate every fragment's text (link
/// labels and inline code included) with all formatting stripped, matching what the user reads.
#[test]
fn plain_text_concatenates_all_fragments_stripping_formatting() {
    let content = BannerTextContent::<TestAction>::formatted_text(vec![
        FormattedTextFragment::plain_text("Seems like your completions are not working ("),
        FormattedTextFragment::hyperlink("more info", "https://example.com"),
        FormattedTextFragment::plain_text("). Enabling the SSH extension may resolve this."),
    ]);

    assert_eq!(
        content.to_plain_text(),
        "Seems like your completions are not working (more info). Enabling the SSH extension may resolve this."
    );
}
