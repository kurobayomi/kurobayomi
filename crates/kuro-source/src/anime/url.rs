//! `getUrlWithoutDomain` for anime (`AnimeHttpSource.kt:685-699`), with `java.net.URI` parity (T-16a-027).
//! Stored rows were written with these rules, so the output must match.

/// The relative form stored for an anime or episode URL. Path, query and fragment come out percent-decoded, as
/// `java.net.URI` returns them. An input that `URI` rejects, and an opaque URI such as `mailto:x`, comes back unchanged
/// with its domain (E-06, E-19, E-20).
pub fn url_without_domain(orig: &str) -> String {
    let Some(parts) = split(orig) else {
        return orig.to_owned();
    };
    let mut out = decode(parts.path);
    if let Some(query) = parts.query {
        out.push('?');
        out.push_str(&decode(query));
    }
    if let Some(fragment) = parts.fragment {
        out.push('#');
        out.push_str(&decode(fragment));
    }
    out
}

/// The manga twin (`HttpSource.getUrlWithoutDomain`, T-16a-044): the same rules, with each space written as `%20` before
/// parsing. A space in the input is therefore accepted and comes back as a space. A rejected input comes back as the
/// original text, spaces included.
pub fn url_without_domain_manga(orig: &str) -> String {
    let replaced = orig.replace(' ', "%20");
    let out = url_without_domain(&replaced);
    if out == replaced { orig.to_owned() } else { out }
}

struct Parts<'a> {
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

/// Splits the raw text the way `URI` does, and returns `None` for every input `URI` would reject or treat as opaque.
fn split(orig: &str) -> Option<Parts<'_>> {
    // Never legal raw, in any component.
    if orig.chars().any(|c| {
        c.is_control() || c.is_whitespace() || matches!(c, '"' | '<' | '>' | '\\' | '^' | '{' | '}' | '|' | '`')
    }) {
        return None;
    }
    let (rest, fragment) = match orig.split_once('#') {
        Some((rest, fragment)) if !fragment.contains('#') => (rest, Some(fragment)),
        Some(_) => return None,
        None => (orig, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    // A scheme is letters, digits, `+`, `-` and `.` before the first colon, and the colon must come before any slash.
    let rest = match rest.split_once(':') {
        Some((scheme, after)) if !scheme.is_empty() && !scheme.contains('/') => {
            let mut chars = scheme.chars();
            let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
            if !valid || !after.starts_with('/') {
                // Opaque (`mailto:x`) or malformed: the original is kept.
                return None;
            }
            after
        }
        _ => rest,
    };
    let (authority, path) = match rest.strip_prefix("//") {
        Some(after) => match after.find('/') {
            Some(at) => (true, &after[at..]),
            None => (true, ""),
        },
        None => (false, rest),
    };
    // `[` and `]` are legal in a query and in a relative path, but not in the path of an absolute URI.
    if authority && path.contains(['[', ']']) {
        return None;
    }
    if !escapes_are_well_formed(path)
        || query.is_some_and(|q| !escapes_are_well_formed(q))
        || fragment.is_some_and(|f| !escapes_are_well_formed(f))
    {
        return None;
    }
    Some(Parts { path, query, fragment })
}

/// Every `%` must start an escape of two hex digits. `URI` rejects anything else.
fn escapes_are_well_formed(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.iter().enumerate().all(|(at, &b)| {
        b != b'%'
            || (bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

/// Percent-decodes to UTF-8 with replacement characters, as `URI.getPath()` does. `+` stays `+`.
fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let (Some(high), Some(low)) =
                (bytes.get(at + 1).and_then(|b| hex(*b)), bytes.get(at + 2).and_then(|b| hex(*b)))
        {
            out.push(high * 16 + low);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::url_without_domain as ani;

    /// Every anime row of the URL compatibility table. A rejected input is returned unchanged.
    #[test]
    fn the_anime_column_of_the_java_vectors() {
        let rows: &[(&str, &str)] = &[
            ("https://aniyomi.org/anime/one-piece", "/anime/one-piece"),
            ("https://aniyomi.org/anime/one-piece?page=2", "/anime/one-piece?page=2"),
            ("https://aniyomi.org/a/b#frag", "/a/b#frag"),
            ("https://aniyomi.org/a%20b/c", "/a b/c"),
            ("https://aniyomi.org/a%26b?q=a%26b", "/a&b?q=a&b"),
            ("https://aniyomi.org/a b/c", "https://aniyomi.org/a b/c"),
            ("https://aniyomi.org/x|y", "https://aniyomi.org/x|y"),
            ("https://aniyomi.org", ""),
            ("https://aniyomi.org/", "/"),
            ("mailto:someone@example.com", "mailto:someone@example.com"),
            ("/relative/path?x=1", "/relative/path?x=1"),
            ("https://aniyomi.org/café/ф", "/café/ф"),
            ("https://aniyomi.org/p?q=1+2", "/p?q=1+2"),
            ("https://aniyomi.org/p?q=%E2%9C%93", "/p?q=✓"),
            ("https://aniyomi.org/p?a=1&b=2#x%20y", "/p?a=1&b=2#x y"),
            ("https://aniyomi.org/with\"quote", "https://aniyomi.org/with\"quote"),
            ("https://aniyomi.org/with^caret", "https://aniyomi.org/with^caret"),
            ("https://aniyomi.org/with{brace}", "https://aniyomi.org/with{brace}"),
            ("https://aniyomi.org/with%zz", "https://aniyomi.org/with%zz"),
            ("http://localhost:1/stream.m3u8?t=1", "/stream.m3u8?t=1"),
            ("https://x.org/p?", "/p?"),
            ("https://x.org/p#", "/p#"),
            ("https://x.org?", "?"),
            ("https://x.org#frag", "#frag"),
            ("https://x.org/a?b=[1]", "/a?b=[1]"),
            ("/a[1]", "/a[1]"),
            ("//host/path?q", "/path?q"),
            ("http://u:p@x.org:8080/p?q#f", "/p?q#f"),
            ("https://x.org/a%2Fb", "/a/b"),
            ("https://x.org/%F0%9F%98%80", "/😀"),
            ("https://x.org/a+b", "/a+b"),
            ("https://x.org/a?b=c d", "https://x.org/a?b=c d"),
            ("x:y", "x:y"),
            ("https://x.org/a\u{a0}b", "https://x.org/a\u{a0}b"),
            ("https://x.org/a%00b", "/a\0b"),
            ("https://x.org/a%E2%9C", "/a\u{fffd}"),
        ];
        for (input, expected) in rows {
            assert_eq!(ani(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn a_bracket_in_the_path_of_an_absolute_uri_is_rejected() {
        assert_eq!(ani("https://x.org/a[b"), "https://x.org/a[b");
    }

    #[test]
    fn a_second_hash_or_a_bar_in_the_fragment_is_rejected() {
        assert_eq!(ani("https://x.org/a#f|g"), "https://x.org/a#f|g");
        assert_eq!(ani("https://x.org/a#f#g"), "https://x.org/a#f#g");
    }

    #[test]
    fn a_stored_relative_url_is_its_own_fixed_point() {
        assert_eq!(ani("/a b/c"), "/a b/c");
        assert_eq!(ani(&ani("https://x.org/a%20b?q=1")), "/a b?q=1");
    }

    #[test]
    fn the_manga_column_of_the_java_vectors() {
        let rows: &[(&str, &str)] = &[
            ("https://aniyomi.org/anime/one-piece", "/anime/one-piece"),
            ("https://aniyomi.org/a%20b/c", "/a b/c"),
            ("https://aniyomi.org/a b/c", "/a b/c"),
            ("https://aniyomi.org/x|y", "https://aniyomi.org/x|y"),
            ("https://aniyomi.org", ""),
            ("mailto:someone@example.com", "mailto:someone@example.com"),
            ("https://aniyomi.org/with\"quote", "https://aniyomi.org/with\"quote"),
            ("https://aniyomi.org/with^caret", "https://aniyomi.org/with^caret"),
            ("https://aniyomi.org/with%zz", "https://aniyomi.org/with%zz"),
            ("https://x.org/a?b=c d", "/a?b=c d"),
            ("https://x.org/a%2Fb", "/a/b"),
            ("https://x.org/a+b", "/a+b"),
            ("x:y", "x:y"),
        ];
        for (input, expected) in rows {
            assert_eq!(super::url_without_domain_manga(input), *expected, "input {input}");
        }
    }
}
