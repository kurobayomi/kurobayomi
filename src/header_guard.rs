//! Guards for the top bar rule (CLAUDE.md): every screen the app pushes has a header bar with the window buttons, so the
//! window can always be dragged, minimised, maximised and closed from it. The display checks build the pages that can be
//! built in a test and look for a header bar in each. The text checks stop a new `ui::push` from pushing a widget that no
//! known builder gives a header, and stop an undecorated window.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gtk::prelude::*;

    use crate::shell::deep_link_page::{self, Reply};
    use kuro_platform::kind::Kind;

    /// Builders whose page carries a header bar: `ui::page`, `page_header::header`, `screens::build`, and the pages the
    /// reader and the player build with `page_header::header`.
    const KNOWN_BUILDERS: [&str; 7] = ["ui::page(", "page_header::header(", "screens::build(", "page_for(", "open_chapter(", "open_local_chapter(", "build_page("];

    /// Pushes of a page that a local helper builds with `ui::page` or `screens::build`, as (file under src, argument).
    const ALLOWED: [(&str, &str); 3] = [
        ("browse/glue.rs", "&page"),                           // the categories page from screens::build
        ("settings_ui/route.rs", "&page"),                     // the categories page from screens::build
        ("settings_ui/licenses.rs", "&detail_page(&library)"), // ui::page
    ];

    fn assert_has_header(what: &str, page: &gtk::Widget) {
        let found = crate::screens::alone::descendants(page).iter().any(|w| w.is::<gtk::HeaderBar>());
        assert!(found, "{what} is pushed without a header bar, so the window cannot be dragged or closed from it");
    }

    #[test]
    fn the_pages_that_can_be_built_in_a_test_have_a_header_bar() {
        crate::testing::with_display(|| {
            let (stack, _window) = crate::testing::nav();
            for id in ["download_queue", "tracking_sheet", "new_update", "crash"] {
                assert_has_header(id, crate::screens::build(id).upcast_ref());
            }
            // the loading page of a deep link: a resolver that never answers keeps it on show
            let never: Rc<dyn Fn(String, Reply)> = Rc::new(|_: String, _: Reply| {});
            deep_link_page::install_resolver(Kind::Manga, never);
            deep_link_page::open(Kind::Manga, "https://example.test/series/1");
            let shown = stack.visible_child().expect("the loading page is on show");
            assert_has_header("the deep-link loading page", &shown);
        });
    }

    #[test]
    fn a_dialog_window_has_its_header_bar_as_its_titlebar() {
        crate::testing::with_display(|| {
            let _ = crate::testing::nav();
            let window = crate::ui::modal(&gtk::Label::new(Some("Title")).upcast(), &gtk::Label::new(Some("Body")), 200, 100);
            assert!(window.titlebar().is_some_and(|bar| bar.is::<gtk::HeaderBar>()), "a dialog has its header bar");
        });
    }

    /// The production part of a source file: everything before its test module.
    fn production(text: &str) -> &str {
        const MARK: &str = "#[cfg(test)]";
        let mut from = 0;
        while let Some(found) = text[from..].find(MARK) {
            let at = from + found;
            let rest = text[at + MARK.len()..].trim_start();
            let rest = rest.strip_prefix("pub(crate) ").or_else(|| rest.strip_prefix("pub ")).unwrap_or(rest);
            // an inline test module ends the production part; `#[cfg(test)] mod x;` (a file) does not
            if let Some(body) = rest.strip_prefix("mod ") {
                let name = body.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(body.len());
                if body[name..].trim_start().starts_with('{') {
                    return &text[..at];
                }
            }
            from = at + MARK.len();
        }
        text
    }

    /// The production source files under `dir`; test files are left out.
    fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the source folder") {
            let path = entry.expect("a folder entry").path();
            if path.is_dir() {
                source_files(&path, out);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            let test_file = name == "tests.rs" || name.ends_with("_tests.rs") || path.components().any(|c| c.as_os_str() == "tests");
            if path.extension().and_then(|e| e.to_str()) == Some("rs") && !test_file {
                out.push(path);
            }
        }
    }

    /// Every `ui::push` call of `text` (`ui::`, `crate::ui::` or the bare name a file imports from `ui`), as (line, argument).
    /// Method calls such as `list.push(..)` and comment lines are skipped.
    fn pushes(text: &str) -> Vec<(usize, String)> {
        let mut found = Vec::new();
        for (at, _) in text.match_indices("push(") {
            let before = &text[..at];
            if before.chars().next_back().is_some_and(|c| c == '.' || c == '_' || c.is_alphanumeric()) {
                continue;
            }
            let line_start = before.rfind('\n').map_or(0, |i| i + 1);
            if text[line_start..at].trim_start().starts_with("//") || before.trim_end().ends_with("fn") {
                continue;
            }
            let mut qualifier: Vec<char> = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':').collect();
            qualifier.reverse();
            let qualifier: String = qualifier.into_iter().collect();
            if !(qualifier.is_empty() || qualifier == "ui::" || qualifier == "crate::ui::") {
                continue;
            }
            let open = at + "push(".len();
            let mut depth = 1usize;
            let mut end = text.len();
            for (i, c) in text[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let arg = text[open..end].split_whitespace().collect::<Vec<_>>().join(" ");
            // a bare `push(..)` is a list or a closure; only `push(&page)` of an imported `ui::push` is a page
            if qualifier.is_empty() && !arg.starts_with('&') {
                continue;
            }
            found.push((before.matches('\n').count() + 1, arg));
        }
        found
    }

    fn src_files() -> Vec<(String, String)> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        source_files(&src, &mut files);
        files
            .into_iter()
            .map(|file| {
                let rel = file.strip_prefix(&src).unwrap_or(file.as_path()).to_string_lossy().replace('\\', "/");
                let text = std::fs::read_to_string(&file).expect("a source file");
                (rel, text)
            })
            .collect()
    }

    #[test]
    fn every_pushed_page_comes_from_a_known_builder_or_is_listed_here() {
        let mut unknown = Vec::new();
        for (rel, text) in src_files() {
            for (line, arg) in pushes(production(&text)) {
                let known = KNOWN_BUILDERS.iter().any(|builder| arg.contains(builder));
                let listed = ALLOWED.iter().any(|(file, allowed)| *file == rel && *allowed == arg);
                if !known && !listed {
                    unknown.push(format!("src/{rel}:{line} pushes `{arg}`"));
                }
            }
        }
        assert!(
            unknown.is_empty(),
            "these pages are pushed without a known header builder (build them with ui::page or page_header::header, or list the call in ALLOWED):\n{}",
            unknown.join("\n")
        );
    }

    #[test]
    fn no_window_is_undecorated_except_the_startup_splash() {
        for (rel, text) in src_files() {
            if rel == "startup.rs" {
                continue;
            }
            assert!(!production(&text).contains("decorated(false)"), "src/{rel} draws an undecorated window, which has no header bar");
        }
    }
}
