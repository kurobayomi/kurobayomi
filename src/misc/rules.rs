//! The content rules every notice passes before the notification service is called (T-35a-003): at most 3 buttons,
//! `&`, `<` and `>` escaped in the title and body, and a body of at most 200 characters.
//!
//! Decision: the desktop services this app reaches (GNOME, KDE and the portal) read the body as a markup subset, and
//! the app cannot ask which one is running, so the text is always escaped.

use super::notify::Post;

/// Freedesktop servers commonly show up to 3 buttons.
pub const MAX_BUTTONS: usize = 3;
/// Longest body, in characters, ellipsis included (PROPOSAL).
pub const MAX_BODY_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyError {
    TooManyButtons,
}

impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyButtons => write!(f, "notice has more than {MAX_BUTTONS} buttons"),
        }
    }
}

pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// `body` cut to [`MAX_BODY_CHARS`] characters, the last one an ellipsis, when it is longer.
pub fn truncate_body(body: &str) -> String {
    if body.chars().count() <= MAX_BODY_CHARS {
        return body.to_owned();
    }
    let mut out: String = body.chars().take(MAX_BODY_CHARS - 1).collect();
    out.push('\u{2026}');
    out
}

/// The notice as it is sent: rejected with more than 3 buttons, otherwise with the body truncated and the title and
/// body escaped (truncation first, so an entity is never cut in half).
pub fn prepare(mut post: Post) -> Result<Post, NotifyError> {
    if post.buttons.len() > MAX_BUTTONS {
        return Err(NotifyError::TooManyButtons);
    }
    post.title = escape_markup(&post.title);
    post.body = post.body.map(|body| escape_markup(&truncate_body(&body)));
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::notify::Target;

    fn post(buttons: usize, title: &str, body: &str) -> Post {
        Post {
            id: "n".into(),
            title: title.into(),
            body: Some(body.into()),
            low_priority: false,
            timeout: None,
            tap: None,
            buttons: (0..buttons).map(|n| (format!("b{n}"), Target::Url("https://x".into()))).collect(),
            icon: None,
        }
    }

    #[test]
    fn four_buttons_are_rejected_and_three_pass() {
        // T-35a-003
        assert_eq!(prepare(post(4, "t", "b")).unwrap_err(), NotifyError::TooManyButtons);
        assert_eq!(prepare(post(3, "t", "b")).unwrap().buttons.len(), 3);
    }

    #[test]
    fn title_and_body_are_escaped() {
        let out = prepare(post(0, "a & <b>", "x < y & z > w")).unwrap();
        assert_eq!(out.title, "a &amp; &lt;b&gt;");
        assert_eq!(out.body.as_deref(), Some("x &lt; y &amp; z &gt; w"));
    }

    #[test]
    fn a_long_body_is_cut_with_an_ellipsis_and_a_200_char_body_is_kept() {
        let kept = "a".repeat(200);
        assert_eq!(truncate_body(&kept), kept);
        let cut = truncate_body(&"é".repeat(201));
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with('\u{2026}'));
        let escaped = prepare(post(0, "t", &format!("{}&", "a".repeat(250)))).unwrap();
        assert!(!escaped.body.unwrap().contains("&amp"), "the cut happens before the escape");
    }
}
