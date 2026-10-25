//! The downloaded titles of one kind, for the Downloads page: the download index holds folder names only (source, title
//! folder, unit folders), so each is matched back to its library entry and items through the same naming rule the
//! downloads are written with (`kuro_downloads::naming`). A title folder or a unit the library no longer knows stays
//! listed, without a library id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_downloads::index::Tree;
use kuro_downloads::naming::{UnitName, entry_dir_name, unit_dir_stems};
use kuro_platform::kind::Kind;

/// One title folder.
#[derive(Debug, PartialEq)]
pub struct Title {
    /// The library entry's id, when the library knows the title.
    pub entry_id: Option<i64>,
    /// The library's title, or the folder name.
    pub label: String,
    pub folder: PathBuf,
    pub units: Vec<Unit>,
}

/// One downloaded episode or chapter.
#[derive(Debug, PartialEq)]
pub struct Unit {
    /// The library item's id, when the library knows the unit.
    pub item_id: Option<i64>,
    /// The item's name, or the folder name.
    pub label: String,
}

/// Every title folder of `tree` (under `root`) that holds a unit, with its units, by title (a folder left empty, by a
/// cancelled download for one, is not listed). `entries_of` gives a source's library entries,
/// `items_of` an entry's items; the units come in the items' order, the unknown ones after them.
pub fn titles(kind: Kind, tree: &Tree, root: &Path, entries_of: impl Fn(i64) -> Vec<Entry>, items_of: impl Fn(i64) -> Vec<Item>) -> Vec<Title> {
    let mut titles = Vec::new();
    for (source_id, source) in &tree.sources {
        let entries: HashMap<String, Entry> = entries_of(*source_id).into_iter().map(|e| (entry_dir_name(&e.title), e)).collect();
        for folder in source.entries.values().filter(|folder| !folder.units.is_empty()) {
            let path = root.join(&folder.source_dir).join(&folder.dir_name);
            let Some(entry) = entries.get(&folder.dir_name) else {
                let units = folder.units.iter().map(|name| Unit { item_id: None, label: name.clone() }).collect();
                titles.push(Title { entry_id: None, label: folder.dir_name.clone(), folder: path, units });
                continue;
            };
            let mut left = folder.units.clone();
            let mut units: Vec<Unit> = items_of(entry.id)
                .into_iter()
                .filter(|item| unit_dir_stems(kind, UnitName::new(&item.name, item.scanlator.as_deref())).iter().any(|stem| left.remove(stem)))
                .map(|item| Unit { item_id: Some(item.id), label: item.name })
                .collect();
            units.extend(left.into_iter().map(|name| Unit { item_id: None, label: name }));
            titles.push(Title { entry_id: Some(entry.id), label: entry.title.clone(), folder: path, units });
        }
    }
    titles.sort_by_key(|title| title.label.to_lowercase());
    titles
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kuro_downloads::index::{EntryDir, SourceDir};

    use super::*;

    fn entry(id: i64, title: &str) -> Entry {
        Entry { id, source: 7, title: title.into(), ..Entry::create(Kind::Anime) }
    }

    fn item(id: i64, name: &str) -> Item {
        Item { id, entry_id: 1, name: name.into(), ..Item::create(Kind::Anime) }
    }

    fn folder(title: &str, units: &[&str]) -> (String, EntryDir) {
        let dir_name = entry_dir_name(title);
        (dir_name.clone(), EntryDir { source_dir: "Source (EN)".into(), dir_name, units: units.iter().map(|u| (*u).to_owned()).collect::<BTreeSet<_>>() })
    }

    #[test]
    fn folders_match_their_library_titles_and_items_and_unknown_ones_stay() {
        let stem = |name: &str| unit_dir_stems(Kind::Anime, UnitName::new(name, None))[0].clone();
        let (one, two) = (stem("Episode 1"), stem("Episode 2"));
        let entries: BTreeMap<String, EntryDir> =
            [folder("Bleach: TYBW", &[&two, &one, "Episode 99 (stray)"]), folder("Gone From Library", &["Episode 1"]), folder("One Piece", &[])].into_iter().collect();
        let tree = Tree { sources: [(7, SourceDir { dir_name: "Source (EN)".into(), entries })].into_iter().collect() };
        let titles = titles(Kind::Anime, &tree, Path::new("/dl"), |_| vec![entry(1, "Bleach: TYBW")], |_| vec![item(10, "Episode 1"), item(11, "Episode 2"), item(12, "Episode 3")]);
        assert_eq!(titles.len(), 2, "an empty title folder is not listed");
        let bleach = &titles[0];
        assert_eq!((bleach.entry_id, bleach.label.as_str()), (Some(1), "Bleach: TYBW"), "the library's title, not the folder name");
        assert_eq!(bleach.folder, Path::new("/dl/Source (EN)").join(entry_dir_name("Bleach: TYBW")));
        let units: Vec<(Option<i64>, &str)> = bleach.units.iter().map(|u| (u.item_id, u.label.as_str())).collect();
        assert_eq!(units, [(Some(10), "Episode 1"), (Some(11), "Episode 2"), (None, "Episode 99 (stray)")], "in the items' order, the unknown unit last");
        assert_eq!((titles[1].entry_id, titles[1].units[0].item_id), (None, None), "a title the library no longer knows stays listed");
    }
}
