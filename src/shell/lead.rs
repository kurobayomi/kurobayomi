//! Guards on the start of the page headers. A tab page has no ⋮ menu: its actions are buttons at the end of its own
//! header (user, 2026-10-25), and every page's search toggle is the first of those actions (user, 2026-10-25). Only a
//! pushed page leads its header with a ⋮ menu (`ui::pack_lead`).

#[cfg(test)]
mod tests {
    /// A tab page has no ⋮ menu (user, 2026-10-25): its actions are buttons in its header. Only pages pushed over the
    /// tabs fill `HeaderActions::menu`: a title's page and the Upcoming page's guide.
    #[test]
    fn no_tab_page_has_a_more_menu() {
        const PUSHED: [&str; 2] = ["src/entry_core/details/route.rs", "src/updates/upcoming/page.rs"];
        fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("the source folder").flatten() {
                let path = entry.path();
                if path.is_dir() { files(&path, out) } else if path.extension().is_some_and(|e| e == "rs") { out.push(path) }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut all = Vec::new();
        files(&root.join("src"), &mut all);
        let mut filled = Vec::new();
        for path in all {
            let name = path.strip_prefix(root).expect("under the crate").to_string_lossy().replace('\\', "/");
            // the pushed pages, the struct itself and this test
            if PUSHED.contains(&name.as_str()) || name == "src/ui.rs" || name == "src/shell/lead.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for (at, _) in text.match_indices("HeaderActions {") {
                // the struct literal, up to its closing brace
                let mut depth = 0;
                let body: String = text[at..]
                    .chars()
                    .take_while(|c| {
                        match c {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        !(depth == 0 && *c == '}')
                    })
                    .collect();
                if body.split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "menu") {
                    filled.push(name.clone());
                }
            }
            if text.contains(".menu.push") {
                filled.push(name);
            }
        }
        assert!(filled.is_empty(), "a tab page fills HeaderActions::menu: {filled:?}");
    }
}
