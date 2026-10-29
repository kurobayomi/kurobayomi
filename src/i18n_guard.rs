//! Guard: the app's own words come from the string table (`kuro-i18n`), never from English text in the code. The test reads
//! the source and fails on
//!
//! 1. a call that puts text on screen (a tooltip, a label, a title, a toast, a button...) with an English string literal, and
//! 2. a `const NAME: &str = "Some English sentence";` (those are shown somewhere, so they belong in the table too), and
//! 3. a `format!` whose text, without its `{}` places, is an English phrase, or starts with a place and reads on in English
//!    ("{name} is not answering"): a sentence with values in it belongs in the table with Fluent arguments.
//!
//! Fix a failure by adding a message to `crates/kuro-i18n/locales/en/kurobayomi-extra.ftl` and to `kurobayomi.ftl` (same
//! text, at the end), then calling `tr(MsgKey::that_key)`. Other languages fall back to English until translated.
//!
//! What is allowed without a key is listed in [`ALLOWED`], each with its reason: text that is not shown to the user as
//! words of the interface (names, identifiers, protocol strings), and words that are the same in every language.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// `(file ending, the const name or the literal)`: English text that may stay in the code, and why.
const ALLOWED: &[(&str, &str, &str)] = &[
    ("src/shell/argv.rs", "USAGE", "the command-line help is for a terminal and is not translated"),
    ("src/pref_model/debug_info.rs", "KERNEL", "a technical label of the debug info, kept in English so reports read the same everywhere"),
    ("src/pref_model/debug_info.rs", "DESKTOP", "as above"),
    ("src/pref_model/debug_info.rs", "SESSION", "as above"),
    ("src/pref_model/debug_info.rs", "GTK", "as above"),
    ("src/reporter.rs", "Kurobayomi", "the name of the app"),
    ("src/shell/sidebar.rs", "Kurobayomi", "the name of the app"),
    ("src/player_view/hls_proxy.rs", "BROWSER_UA", "an HTTP user agent, not shown"),
    ("src/channel.rs", "REPOSITORY", "a GitHub owner/name, not shown"),
    ("src/reader/viewers/nav.rs", "*", "overlay labels as message key plus English fallback"),
    ("src/shell/desktop_labels.rs", "*", "the table of desktop action keys: each row names its message keys and keeps the English as a fallback"),
    ("src/library_update/store.rs", "*", "SQL"),
    ("src/settings.rs", "SELECT count(*) FROM items i JOIN entries e ON e._id = i.entry_id WHERE e.kind = ?1", "SQL"),
    ("src/player_view/hls_proxy.rs", "*", "HTTP bytes of the local relay"),
    ("src/shell/intents/drops.rs", "*", "the registry of Android features dropped on Linux: notes for developers, not interface text"),
    ("src/dropped/", "*", "as above"),
    ("src/browse/fake_catalogue.rs", "*", "a fake catalogue for development and tests"),
    ("src/channel.rs", "Kurobayomi Nightly", "the name of the app"),
    ("src/ui.rs", "Clear WebView data", "compared with the English title of a spec action (the row shows the translated title)"),
    ("src/settings_ui/route.rs", "Tracking guide", "compared with the English tooltip of a spec button"),
    ("src/updates/mount.rs", "Update library", "the English label of a spec toolbar item, compared to find it"),
    ("src/updates/mount.rs", "View Upcoming Updates", "as above"),
    ("src/updates/mount.rs", "UPDATE_LIBRARY", "the English label of a spec toolbar item, compared to find it (spec/*.json holds the text shown)"),
    ("src/updates/mount.rs", "VIEW_UPCOMING", "as above"),
    ("src/pref_model/stores_page.rs", "Discord", "the name of a product, the same in every language"),
    ("src/settings_ui/worker_info.rs", "*", "the job dump, printed as Android prints it, for developers"),
    ("src/shell/search_provider.rs", "*", "the desktop's search provider file, not shown"),
    ("src/pref_model/debug_info.rs", "*", "the debug info, kept in English so reports read the same everywhere"),
    ("src/settings_ui/menu.rs", "{app} Stable {version}", "a build name: the app, its channel and version"),
    ("src/settings_ui/menu.rs", "Debug {commit}", "a build name"),
    ("src/whats_new.rs", "Nightly {}", "a build name: the channel and its commit"),
    ("src/whats_new.rs", "Kurobayomi {version}", "a build name: the app and its version"),
    ("src/library_update/fake.rs", "*", "a fake library for development and tests"),
    ("src/cli.rs", "*", "the command line's own messages, for a terminal"),
    ("src/misc/headless.rs", "*", "as above"),
];

/// Calls that show their argument to the user.
const SINK: &str = r#"(?:icon_button\([^,()]+,\s*|refresh_button\(|set_tooltip_text\(Some\(|tooltip_text\(|Label::new\(Some\(|Label::builder\(\)\.label\(|\.label\(|with_label\(|heading\(|ui::dim\(|\bdim\(|placeholder_text\(|set_label\(|set_title\(|\.title\(|toast::notify\(|notify\(|DialogButton::\w+\(|Toaster::show\(|\.show\(|\btext\(|tap_item\(|switch_item\(|set_text\()\s*&?\s*"((?:[^"\\]|\\.)+)""#;

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            // files that are only tests, and this guard itself
            if !(name.contains("test") || name == "i18n_guard.rs" || path.components().any(|c| c.as_os_str() == "testing" || c.as_os_str() == "t2")) {
                out.push(path);
            }
        }
    }
}

/// The code of a file without comments and without the items behind `#[cfg(test)]` (test modules and test-only items).
fn code_of(text: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            // skip the attribute and the item it applies to: a braced block (a module, a fn) or a one-line item
            i += 1;
            while i < lines.len() && lines[i].trim_start().starts_with("#[") {
                i += 1;
            }
            let mut depth = 0i32;
            let mut seen_brace = false;
            while i < lines.len() {
                for c in lines[i].chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            seen_brace = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                let done = if seen_brace { depth <= 0 } else { lines[i].trim_end().ends_with(';') };
                i += 1;
                if done {
                    break;
                }
            }
            continue;
        }
        if !trimmed.starts_with("//") {
            out.push_str(line);
            out.push('\n');
        }
        i += 1;
    }
    out
}

/// Whether a literal is English words for the interface: a capital letter first and some lower-case letters after.
fn is_english_words(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && text.chars().filter(|c| c.is_ascii_lowercase()).count() >= 2
        && !text.contains("::")
        && !text.starts_with("http")
}

/// Lines that are for developers, not the interface: logging, assertions, panics, attributes and embedded files.
const QUIET: &[&str] = &["tracing::", "debug!(", "info!(", "warn!(", "error!(", "trace!(", "expect(", "panic!(", "unreachable!(", "assert", "eprintln!(", "println!(", "include_str!(", "#[", "bail!("];

/// Whether a literal reads as an English phrase for the interface: a capital first, a space, and some lower-case letters.
fn is_english_phrase(text: &str) -> bool {
    is_english_words(text) && text.contains(' ') && text.len() >= 5 && !text.contains('{') && !text.starts_with("Mozilla")
}

fn unescape(text: &str) -> String {
    text.replace("\\\"", "\"").replace("\\'", "'")
}

fn allowed(file: &str, what: &str) -> bool {
    ALLOWED.iter().any(|(f, w, _)| (file.ends_with(f) || (f.ends_with('/') && file.contains(f))) && (*w == "*" || *w == what))
}

#[test]
fn the_interface_text_comes_from_the_string_table() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    source_files(&root.join("src"), &mut files);
    assert!(files.len() > 100, "the guard found the sources");
    let sink = Regex::new(SINK).expect("a valid pattern");
    let constant = Regex::new(r#"(?m)\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*\n?\s*"((?:[^"\\]|\\.)*)"\s*;"#).expect("a valid pattern");
    // the settings row builders take the row's title first, and option lists pair a stored value with its label
    let builder = Regex::new(r#"\b(?:group|switch|switch_with|exclusive_switch|ranged_choice|multi_choice|int_choice|text|info|edit_text|tap_item|list_row|choice|slider)\(\s*&?"((?:[^"\\]|\\.)+)""#).expect("a valid pattern");
    let option = Regex::new(r#"\(\s*(?:-?\d+|"[a-z0-9_.\-]+")\s*,\s*"((?:[^"\\]|\\.)+)"\s*\)"#).expect("a valid pattern");
    let literal = Regex::new(r#""((?:[^"\\]|\\.)+)""#).expect("a valid pattern");
    let formatted = Regex::new(r#"format!\(\s*"((?:[^"\\]|\\.)+)""#).expect("a valid pattern");
    let places = Regex::new(r"\{[^}]*\}").expect("a valid pattern");
    let place_first = Regex::new(r"^\{[^}]*\}\s+[a-z']+ [a-z']+ [a-z']+").expect("a valid pattern");
    let mut found = Vec::new();
    for path in files {
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let code = code_of(&fs::read_to_string(&path).expect("a readable source"));
        for m in sink.captures_iter(&code) {
            let text = unescape(&m[1]);
            if is_english_words(&text) && !allowed(&relative, &text) {
                found.push(format!("{relative}: shown text {text:?}"));
            }
        }
        for rule in [&builder, &option] {
            for m in rule.captures_iter(&code) {
                let text = unescape(&m[1]);
                if is_english_words(&text) && !allowed(&relative, &text) && !found.iter().any(|f: &String| f.ends_with(&format!("{text:?}"))) {
                    found.push(format!("{relative}: row text {text:?}"));
                }
            }
        }
        // any English phrase in the code, whatever call it is in (a line that logs, asserts or panics is for developers)
        for line in code.lines() {
            if QUIET.iter().any(|q| line.contains(q)) {
                continue;
            }
            for m in literal.captures_iter(line) {
                let text = unescape(&m[1]);
                if is_english_phrase(&text) && !allowed(&relative, &text) {
                    found.push(format!("{relative}: phrase {text:?}"));
                }
            }
        }
        // a sentence built with format!, on lines that are not for developers
        for line in code.lines().filter(|line| !QUIET.iter().any(|q| line.contains(q))) {
            for m in formatted.captures_iter(line) {
                let text = unescape(&m[1]);
                let bare = places.replace_all(&text, "");
                if (is_english_phrase(bare.trim_start_matches(|c: char| !c.is_ascii_alphabetic())) || place_first.is_match(&text)) && !allowed(&relative, &text) {
                    found.push(format!("{relative}: formatted {text:?}"));
                }
            }
        }
        for m in constant.captures_iter(&code) {
            let (name, text) = (&m[1], unescape(&m[2]));
            let sentence = is_english_words(&text) && (text.contains(' ') || text.ends_with(['.', '…', '?', '!', ':']));
            if sentence && !allowed(&relative, name) {
                found.push(format!("{relative}: const {name} = {text:?}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "English text in the code: move each into the string table (see the top of src/i18n_guard.rs):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_cfg_test_items_are_left_out_of_the_check() {
    let code = code_of("fn a() { x.label(\"Shown\"); }\n#[cfg(test)]\nmod tests {\n    fn b() { x.label(\"Test only\"); }\n}\n// x.label(\"Comment\")\nfn c() {}\n#[cfg(test)]\nconst T: &str = \"Test const\";\nfn d() {}\n");
    assert!(code.contains("Shown") && code.contains("fn c()") && code.contains("fn d()"));
    assert!(!code.contains("Test only") && !code.contains("Comment") && !code.contains("Test const"), "{code}");
}

#[test]
fn a_formatted_sentence_is_caught_and_values_alone_are_not() {
    let places = Regex::new(r"\{[^}]*\}").expect("a valid pattern");
    let bare = places.replace_all("Cache cleared, {n} files deleted", "");
    assert!(is_english_phrase(&bare));
    assert!(!is_english_phrase(&places.replace_all("{}: {}%", "")));
}

#[test]
fn english_words_are_told_from_identifiers_and_urls() {
    assert!(is_english_words("Check for updates") && is_english_words("Delete"));
    assert!(!is_english_words("view-refresh-symbolic") && !is_english_words("https://example.org") && !is_english_words("GTK"));
    assert!(!is_english_words("kb-status") && !is_english_words("io.github.anas1412.Kurobayomi"));
}
