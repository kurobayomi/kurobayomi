//! The markdown that sources put in descriptions (Mihon renders it), turned into Pango markup for a `gtk::Label`:
//! links, bold, italic, code, headings (bold) and `-`/`*`/`+` lists (bullets). Anything else stays as written. A text
//! the converter cannot turn into valid markup is shown escaped, never broken. Some sources send HTML instead
//! (`<b>`, `<br>`, `<a href>`): it is turned into the same markdown first ([`html_to_markdown`]), so it goes through the
//! same escaping and a source can still put no markup of its own on screen.

use std::sync::LazyLock;

use gtk::glib;
use regex::Regex;

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$").unwrap());
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)[-*+]\s+").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\((https?://[^)\s]+)\)").unwrap());
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(\S(?:.*?\S)?)\*\*|__(\S(?:.*?\S)?)__").unwrap());
static ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|[\s(])[*_](\S(?:[^*_]*?\S)?)[*_]").unwrap());
static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
/// A tag of the HTML that some sources send; text without one is not HTML ("I <3 this" stays as written).
static HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:br|p|b|i|a|strong|em|div|span|u|ul|ol|li|h[1-6])\b[^>]*>").unwrap());
static HTML_BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[ \t]*(?:<br\s*/?>|</p>|</div>|</li>|</h[1-6]>)[ \t]*\r?\n?").unwrap());
static HTML_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>\s*").unwrap());
static HTML_BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(?:b|strong)\b[^>]*>\s*(.*?)\s*</(?:b|strong)>").unwrap());
static HTML_ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(?:i|em)\b[^>]*>\s*(.*?)\s*</(?:i|em)>").unwrap());
static HTML_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a\b[^>]*?href\s*=\s*["']?([^"'\s>]*)["']?[^>]*>(.*?)</a>"#).unwrap());
static HTML_ANY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static HTML_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").unwrap());

/// `text` as Pango markup.
pub fn to_markup(text: &str) -> String {
    let text = &html_to_markdown(text);
    let markup = text.lines().map(line).collect::<Vec<_>>().join("\n");
    if is_valid(&markup) { markup } else { glib::markup_escape_text(text).to_string() }
}

/// A description written in HTML as the markdown [`to_markup`] reads: breaks and paragraphs become lines, list items
/// bullets, bold and italic their marks, a web link `[text](url)` (any other link only its text). Every other tag is
/// dropped and the entities are decoded. Text with no HTML tag comes back as it is.
pub fn html_to_markdown(text: &str) -> String {
    if !HTML_TAG.is_match(text) {
        return text.to_owned();
    }
    let text = HTML_BREAK.replace_all(text, "\n");
    let text = HTML_ITEM.replace_all(&text, "- ");
    let text = HTML_LINK.replace_all(&text, |c: &regex::Captures| {
        let label = HTML_ANY.replace_all(&c[2], "").trim().to_owned();
        if c[1].starts_with("https://") || c[1].starts_with("http://") { format!("[{label}]({})", &c[1]) } else { label }
    });
    let text = HTML_BOLD.replace_all(&text, "**$1**");
    let text = HTML_ITALIC.replace_all(&text, "*$1*");
    let text = HTML_ANY.replace_all(&text, "");
    let text = HTML_NUMBER.replace_all(&text, |c: &regex::Captures| {
        let code = c[1].strip_prefix('x').map_or_else(|| c[1].parse().ok(), |hex| u32::from_str_radix(hex, 16).ok());
        code.and_then(char::from_u32).map_or_else(|| c[0].to_owned(), String::from)
    });
    // &amp; last, so an escaped entity ("&amp;lt;") reads as written
    text.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&").trim().to_owned()
}

/// Pango does not know `<a>` (only a GTK label does), so links are checked as spans.
fn is_valid(markup: &str) -> bool {
    let spans = markup.replace("</a>", "</span>").replace("<a href=", "<span font_features=");
    gtk::pango::parse_markup(&spans, '\0').is_ok()
}

fn line(raw: &str) -> String {
    let escaped = glib::markup_escape_text(raw).to_string();
    if let Some(heading) = HEADING.captures(&escaped) {
        return format!("<b>{}</b>", inline(&heading[1]));
    }
    let bulleted = BULLET.replace(&escaped, "$1• ");
    inline(&bulleted)
}

fn inline(text: &str) -> String {
    let text = CODE.replace_all(text, "<tt>$1</tt>");
    let text = LINK.replace_all(&text, r#"<a href="$2">$1</a>"#);
    let text = BOLD.replace_all(&text, "<b>$1$2</b>");
    ITALIC.replace_all(&text, "$1<i>$2</i>").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_bold_italic_lists_and_headings_become_markup() {
        assert_eq!(to_markup("- [AniList](https://anilist.co/manga/46765)"), r#"• <a href="https://anilist.co/manga/46765">AniList</a>"#);
        assert_eq!(to_markup("**Bold** and *italic* and _this_"), "<b>Bold</b> and <i>italic</i> and <i>this</i>");
        assert_eq!(to_markup("## Synopsis"), "<b>Synopsis</b>");
        assert_eq!(to_markup("a `code` b"), "a <tt>code</tt> b");
    }

    #[test]
    fn a_description_in_html_shows_its_lines_bold_italic_and_web_links_and_no_tags() {
        let html = "<b>From the creator of Time Stop Hero</b><br>\n<br>\nAfter losing her father...<br>\n<i>Source: Kodansha USA</i><br>\nOfficial Raw: <a href=\"https://manga.nicovideo.jp/comic/69346\" target=\"_blank\" rel=\"noopener\">Suiyoubi no Sirius</a>, <a href=\"javascript:alert(1)\">Bad</a> &amp; more";
        assert_eq!(
            html_to_markdown(html),
            "**From the creator of Time Stop Hero**\n\nAfter losing her father...\n*Source: Kodansha USA*\nOfficial Raw: [Suiyoubi no Sirius](https://manga.nicovideo.jp/comic/69346), Bad & more"
        );
        let markup = to_markup(html);
        assert!(markup.starts_with("<b>From the creator of Time Stop Hero</b>\n\n"), "{markup}");
        assert!(markup.contains(r#"<a href="https://manga.nicovideo.jp/comic/69346">Suiyoubi no Sirius</a>"#), "{markup}");
        assert!(!markup.contains("javascript") && markup.contains("Bad &amp; more"), "{markup}");
        assert_eq!(html_to_markdown("<ul><li>One</li><li>Two</li></ul>"), "- One\n- Two");
        assert_eq!(html_to_markdown("I <3 this, a &lt; b"), "I <3 this, a &lt; b", "no tag: not HTML");
    }

    #[test]
    fn plain_text_and_special_characters_stay_as_written() {
        assert_eq!(to_markup("Tom & Jerry <3, 2*3*4, snake_case_name"), "Tom &amp; Jerry &lt;3, 2*3*4, snake_case_name");
        assert_eq!(to_markup("a [link](https://x.org/?a=1&b=2)"), r#"a <a href="https://x.org/?a=1&amp;b=2">link</a>"#);
        assert!(is_valid(&to_markup("**unclosed *mix** of_ marks_")));
    }
}
