//! AniList descriptions are HTML. The Kotlin code used `Html.fromHtml`. The Rust
//! version breaks lines at `<br>`, strips every tag, then decodes entities (E-36).

/// Plain text of an AniList description.
pub fn plain_text(description: &str) -> String {
    let with_breaks = description
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let stripped = ammonia::Builder::empty().clean(&with_breaks).to_string();
    html_escape::decode_html_entities(&stripped)
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_28a_029_breaks_tags_and_entities() {
        let input = "Line one<br>Line <i>two</i> &amp; &#39;three&#39;<br />end";
        assert_eq!(plain_text(input), "Line one\nLine two & 'three'\nend");
    }

    #[test]
    fn t_28a_029_plain_text_is_kept() {
        assert_eq!(plain_text("  just words  "), "just words");
    }
}
