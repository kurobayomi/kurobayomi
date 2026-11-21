//! AniList's HTML of a comment or a review, as the markup of one GTK label (docs/design/comments.md, "Showing AniList
//! text"). AniList renders its Markdown itself (`asHtml: true`), so this reads a short list of HTML tags and drops the
//! rest, keeping their text:
//!
//! - bold, italic, strikethrough, underline, code, headings (bold), quotes (italic) and list items (`• `) keep their
//!   look; paragraphs, line breaks, rules, centred and other blocks become line breaks;
//! - a web link stays a link; an image, a video or a YouTube block becomes a link named "Image" or "Video" (no image
//!   loading in a comment list);
//! - a spoiler the author tagged (`<span class='markdown_spoiler'>`) becomes a grey "Spoiler" link until it is clicked:
//!   [`spoiler_index`] reads the click, and the label is drawn again with that spoiler in `revealed`.
//!
//! The markup is built tag by tag on a stack, so it always nests, and every piece of text is escaped: nothing AniList
//! sends reaches the label as markup. AniList's HTML is not always well formed (an unclosed `<center>`, `<h1><center>`
//! closed by `</h1>`), which the stack absorbs.

use std::collections::HashSet;

use kuro_i18n::{MsgKey, tr};

/// A text longer than this, in characters or in lines, is cut with "Show more".
pub const LONG_CHARS: usize = 600;
pub const LONG_LINES: usize = 12;

/// The link a hidden spoiler carries: `spoiler:<index>`.
const SPOILER_SCHEME: &str = "spoiler:";

/// One rendering of a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// GTK label markup (Pango markup with `<a href>` links).
    pub markup: String,
    /// How many spoilers the text has, revealed or not.
    pub spoilers: usize,
    /// The text is long enough to be cut ([`LONG_CHARS`], [`LONG_LINES`]).
    pub long: bool,
}

/// The spoiler a clicked link opens, or `None` for a web link.
pub fn spoiler_index(uri: &str) -> Option<usize> {
    uri.strip_prefix(SPOILER_SCHEME)?.parse().ok()
}

/// One open element: the HTML tag that opened it and the markup tag it opened (`None` for one that only groups).
#[derive(Clone, Copy)]
struct Open {
    html: &'static str,
    markup: Option<&'static str>,
}

struct Writer<'a> {
    out: String,
    stack: Vec<Open>,
    revealed: &'a HashSet<usize>,
    spoilers: usize,
    /// While above zero, a hidden spoiler's content is skipped: its depth of `<span>`.
    hidden_depth: usize,
    /// The `<span>`s open, each marking whether it opened a spoiler.
    spans: Vec<bool>,
    /// Characters of text written so far (a hidden spoiler's are not).
    chars: usize,
}

/// The tags read, each with the markup it opens.
fn inline_markup(tag: &str) -> Option<Option<&'static str>> {
    Some(Some(match tag {
        "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "b",
        "i" | "em" | "blockquote" => "i",
        "s" | "del" | "strike" => "s",
        "u" | "ins" => "u",
        "code" | "pre" => "tt",
        _ => return None,
    }))
}

/// The tags that start and end a block: a line break before and after.
fn is_block(tag: &str) -> bool {
    matches!(tag, "p" | "div" | "center" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "ul" | "ol" | "li" | "pre" | "hr" | "br" | "table" | "tr")
}

/// The `'static` name of a tag the writer keeps on its stack.
fn static_name(tag: &str) -> &'static str {
    const NAMES: [&str; 22] = [
        "b", "strong", "h1", "h2", "h3", "h4", "h5", "h6", "i", "em", "blockquote", "s", "del", "strike", "u", "ins", "code", "pre", "a", "span", "center", "div",
    ];
    NAMES.iter().find(|n| **n == tag).copied().unwrap_or("")
}

impl Writer<'_> {
    fn hidden(&self) -> bool {
        self.hidden_depth > 0
    }

    fn in_link(&self) -> bool {
        self.stack.iter().any(|o| o.markup == Some("a"))
    }

    /// Ends the line, keeping at most one empty line between blocks and none at the start.
    fn line_break(&mut self, lines: usize) {
        let trimmed = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(trimmed);
        // nothing shown yet: no empty lines at the top
        if self.chars == 0 {
            return;
        }
        let have = self.out.chars().rev().take_while(|c| *c == '\n').count();
        for _ in have..lines.min(2) {
            self.out.push('\n');
        }
    }

    fn text(&mut self, raw: &str) {
        if self.hidden() {
            return;
        }
        let decoded = decode_entities(raw);
        let mut text = decoded.as_str();
        // the line breaks between AniList's blocks are already drawn by the blocks
        if self.out.is_empty() || self.out.ends_with('\n') {
            text = text.trim_start();
        }
        self.chars += text.chars().count();
        self.out.push_str(&escape(text));
    }

    /// Opens an element: writes `open` (its markup tag, or nothing for one that only groups) and keeps it on the stack.
    fn push(&mut self, html: &'static str, markup: Option<&'static str>, open: &str) {
        self.out.push_str(open);
        self.stack.push(Open { html, markup });
    }

    /// Closes the element `html` opened: what was opened inside it is closed first and opened again after, so the
    /// markup stays nested. A closing tag with nothing open for it is dropped.
    fn close(&mut self, html: &str) {
        let Some(at) = self.stack.iter().rposition(|o| o.html == html) else { return };
        let above: Vec<Open> = self.stack.drain(at + 1..).collect();
        for open in above.iter().rev() {
            close_markup(&mut self.out, open.markup);
        }
        let closed = self.stack.pop();
        close_markup(&mut self.out, closed.and_then(|o| o.markup));
        for open in above {
            // a link is not opened again: its address is gone
            if let Some(tag) = open.markup.filter(|t| *t != "a") {
                self.out.push_str(&format!("<{tag}>"));
                self.stack.push(open);
            }
        }
    }

    /// A link named `name` to `url` (an image or a video), when `url` is a web address and no link is open.
    fn media_link(&mut self, url: Option<&str>, name: &str) {
        if self.hidden() {
            return;
        }
        let url = url.filter(|u| is_web(u));
        match url {
            Some(url) if !self.in_link() => self.out.push_str(&format!("<a href=\"{}\">{}</a>", escape(url), escape(name))),
            _ => self.out.push_str(&escape(name)),
        }
        self.out.push(' ');
        self.chars += name.chars().count();
    }

    fn open_tag(&mut self, name: &str, attributes: &str) {
        if name == "span" {
            let spoiler = attribute(attributes, "class").is_some_and(|c| c.split_whitespace().any(|c| c == "markdown_spoiler"));
            if self.hidden() {
                self.hidden_depth += 1;
                self.spans.push(false);
                return;
            }
            self.spans.push(spoiler);
            if spoiler {
                let index = self.spoilers;
                self.spoilers += 1;
                if !self.revealed.contains(&index) {
                    self.out.push_str(&format!(
                        "<a href=\"{SPOILER_SCHEME}{index}\"><span background=\"#77767b\" foreground=\"#ffffff\"> {} </span></a>",
                        escape(&tr(MsgKey::comments_spoiler))
                    ));
                    self.hidden_depth = 1;
                }
            }
            return;
        }
        if self.hidden() {
            return;
        }
        match name {
            "img" => return self.media_link(attribute(attributes, "src").as_deref(), &tr(MsgKey::comments_image)),
            "video" | "source" => return self.media_link(attribute(attributes, "src").as_deref(), &tr(MsgKey::comments_video)),
            "div" if attribute(attributes, "class").is_some_and(|c| c.contains("youtube")) => {
                self.line_break(1);
                return self.media_link(attribute(attributes, "id").as_deref(), &tr(MsgKey::comments_video));
            }
            _ => {}
        }
        if is_block(name) {
            self.line_break(if matches!(name, "br" | "li") { 1 } else { 2 });
        }
        if name == "li" {
            self.out.push_str("\u{2022} ");
        }
        if name == "a" {
            match attribute(attributes, "href").filter(|h| is_web(h)) {
                Some(href) if !self.in_link() => self.push("a", Some("a"), &format!("<a href=\"{}\">", escape(&href))),
                _ => self.push("a", None, ""),
            }
            return;
        }
        if let Some(markup) = inline_markup(name) {
            let tag = static_name(name);
            self.push(tag, markup, &markup.map(|m| format!("<{m}>")).unwrap_or_default());
        } else if matches!(name, "center" | "div") {
            self.push(static_name(name), None, "");
        }
    }

    fn close_tag(&mut self, name: &str) {
        if name == "span" {
            self.spans.pop();
            if self.hidden() {
                self.hidden_depth -= 1;
            }
            return;
        }
        if self.hidden() {
            return;
        }
        self.close(name);
        if is_block(name) {
            self.line_break(if name == "li" { 1 } else { 2 });
        }
    }

    fn finish(mut self) -> Rendered {
        while let Some(open) = self.stack.pop() {
            close_markup(&mut self.out, open.markup);
        }
        let markup = collapse_breaks(self.out.trim());
        let lines = markup.matches('\n').count() + 1;
        Rendered { long: self.chars > LONG_CHARS || lines > LONG_LINES, spoilers: self.spoilers, markup }
    }
}

fn close_markup(out: &mut String, markup: Option<&str>) {
    if let Some(tag) = markup {
        out.push_str(&format!("</{tag}>"));
    }
}

/// At most one empty line in a row.
fn collapse_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = 0;
    for c in text.chars() {
        if c == '\n' {
            run += 1;
            if run > 2 {
                continue;
            }
        } else {
            run = 0;
        }
        out.push(c);
    }
    out
}

fn is_web(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// The value of `name` in a tag's attributes (`name='v'`, `name="v"` or `name=v`), entities decoded.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let lower = attributes.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let starts_word = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = attributes[from..].trim_start();
        if !starts_word || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('\'' | '"')) => rest[1..].split(quote).next().unwrap_or_default(),
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
        };
        return Some(decode_entities(value));
    }
    None
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`, `&apos;`, `&nbsp;` and numeric entities; anything else stays as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(';').map(|i| i + 1).filter(|i| *i <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" | "#39" => '\'',
                "nbsp" => ' ',
                _ => {
                    let code = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")).map(|hex| u32::from_str_radix(hex, 16)).or_else(|| name.strip_prefix('#').map(str::parse))?.ok()?;
                    char::from_u32(code)?
                }
            };
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text made safe for label markup.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `html` as label markup, with the spoilers whose index is in `revealed` shown.
pub fn render(html: &str, revealed: &HashSet<usize>) -> Rendered {
    let mut writer = Writer { out: String::new(), stack: Vec::new(), revealed, spoilers: 0, hidden_depth: 0, spans: Vec::new(), chars: 0 };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(at) = rest.find('<') else {
            writer.text(rest);
            break;
        };
        writer.text(&rest[..at]);
        rest = &rest[at..];
        // a `<` that opens no tag ("a <3 b") is text
        let Some(end) = rest.find('>') else {
            writer.text(rest);
            break;
        };
        let inside = &rest[1..end];
        let closing = inside.starts_with('/');
        let body = inside.trim_start_matches('/').trim_end_matches('/');
        let name_len = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
        let name = body[..name_len].to_ascii_lowercase();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            writer.text(&rest[..1]);
            rest = &rest[1..];
            continue;
        }
        if closing {
            writer.close_tag(&name);
        } else {
            writer.open_tag(&name, &body[name_len..]);
        }
        rest = &rest[end + 1..];
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(html: &str) -> String {
        render(html, &HashSet::new()).markup
    }

    /// The markup with its links taken out, as Pango reads it (`<a>` is the label's own addition).
    fn pango_check(markup: &str) {
        let without_links = regex::Regex::new(r#"</?a( href="[^"]*")?>"#).unwrap().replace_all(markup, "");
        gtk::pango::parse_markup(&without_links, '\0').unwrap_or_else(|e| panic!("{e}: {markup}"));
    }

    #[test]
    fn paragraphs_and_breaks_become_lines_without_extra_blank_lines() {
        let html = "<p>Coldest moment<br />\nMusic was amazing too</p>\n<p>Second</p>";
        assert_eq!(plain(html), "Coldest moment\nMusic was amazing too\n\nSecond");
    }

    #[test]
    fn the_look_of_the_text_is_kept_and_escaped() {
        let markup = plain("<p>your life <strong>literally</strong> &amp; <em>a</em> <del>b</del> &lt;3 &quot;x&quot; &#39;y&#39;</p>");
        assert_eq!(markup, "your life <b>literally</b> &amp; <i>a</i> <s>b</s> &lt;3 &quot;x&quot; &#39;y&#39;");
        pango_check(&markup);
    }

    #[test]
    fn a_hidden_spoiler_is_a_link_and_its_text_is_not_in_the_markup() {
        let html = "<p>I need to see <span class='markdown_spoiler'><span>The Golden City arc </span></span>in anime form.</p>";
        let hidden = render(html, &HashSet::new());
        assert_eq!(hidden.spoilers, 1);
        assert!(hidden.markup.contains("<a href=\"spoiler:0\">"), "{}", hidden.markup);
        assert!(!hidden.markup.contains("Golden"), "the spoiler's words never reach the label");
        assert!(hidden.markup.ends_with("in anime form."));
        pango_check(&hidden.markup);
        let shown = render(html, &HashSet::from([0]));
        assert!(shown.markup.contains("The Golden City arc"));
        assert!(!shown.markup.contains("spoiler:"));
    }

    #[test]
    fn spoilers_are_counted_in_order_and_open_one_by_one() {
        let html = "<span class='markdown_spoiler'><span>one</span></span> and <span class='markdown_spoiler'><span>two</span></span>";
        let second = render(html, &HashSet::from([1]));
        assert_eq!(second.spoilers, 2);
        assert!(second.markup.contains("spoiler:0") && second.markup.contains("two") && !second.markup.contains("one"));
        assert_eq!(spoiler_index("spoiler:1"), Some(1));
        assert_eq!(spoiler_index("https://anilist.co"), None);
    }

    #[test]
    fn images_and_videos_become_links() {
        let markup = plain("<p>LET FRIEREN COOK!!</p>\n<img width='512' src='https://i.imgur.com/AIfKnU4.jpg'>");
        assert_eq!(markup, "LET FRIEREN COOK!!\n\n<a href=\"https://i.imgur.com/AIfKnU4.jpg\">Image</a>");
        let video = plain("<center><div class='youtube' id='https://youtu.be/OIBODIPC_8Y?si=RX'></div></center>");
        assert_eq!(video, "<a href=\"https://youtu.be/OIBODIPC_8Y?si=RX\">Video</a>");
        assert_eq!(plain("<img src='file:///etc/passwd'>"), "Image", "only web addresses become links");
    }

    #[test]
    fn links_keep_their_address_escaped_and_never_nest() {
        let markup = plain("<a href=\"https://a.co/?x=1&amp;y=2\">see <a href='https://b.co'>this</a></a>");
        assert_eq!(markup, "<a href=\"https://a.co/?x=1&amp;y=2\">see this</a>");
        assert_eq!(plain("<a href='javascript:alert(1)'>x</a>"), "x");
    }

    #[test]
    fn broken_html_still_nests() {
        let markup = plain("<h1><center>Frieren #1: Journey's End</h1>\n<hr />\n<p>SB: <b>Keiichiro <i>Saito</b> rest</i></p>");
        pango_check(&markup);
        assert!(markup.starts_with("<b>Frieren #1: Journey&#39;s End</b>"), "{markup}");
        let unclosed = plain("<center>__Get yourself a wife__ <b>bold");
        assert_eq!(unclosed, "__Get yourself a wife__ <b>bold</b>");
    }

    #[test]
    fn unknown_tags_go_and_their_text_stays() {
        assert_eq!(plain("<p>a <sup>b</sup> <marquee>c</marquee></p>"), "a b c");
        assert_eq!(plain("a <3 b"), "a &lt;3 b", "a lone < is text");
        assert_eq!(plain("x &bogus; y"), "x &amp;bogus; y");
    }

    #[test]
    fn lists_get_bullets() {
        assert_eq!(plain("<ul><li>one</li><li>two</li></ul>"), "\u{2022} one\n\u{2022} two");
    }

    #[test]
    fn a_long_text_is_marked_long() {
        assert!(!render("<p>short</p>", &HashSet::new()).long);
        assert!(render(&format!("<p>{}</p>", "a".repeat(LONG_CHARS + 1)), &HashSet::new()).long);
        assert!(render(&"<p>x</p>".repeat(LONG_LINES), &HashSet::new()).long);
    }
}
