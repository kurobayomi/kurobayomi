//! Pure queue model for the Download queue page (T-25a-020, T-25a-031).
//!
//! The engine only knows the flat queue. The page groups it by source, moves rows inside a group, then sends the flat
//! order back. No GTK and no engine here.

use std::sync::Arc;

use kuro_platform::kind::Kind;

use crate::unit::QueuedUnit;
use crate::model::DownloadState;

/// One row of the queue. `parent_id` is the series key (anime id or manga id).
#[derive(Clone, Debug, PartialEq)]
pub struct QueueItem {
    pub kind: Kind,
    /// Episode id or chapter id.
    pub item_id: i64,
    pub parent_id: i64,
    pub source_id: i64,
    pub source_name: String,
    /// The entry title (anime or manga), shown as the row subtitle.
    pub title: String,
    /// The episode or chapter name, shown as the row title.
    pub item_name: String,
    /// Sort key "By upload date".
    pub date_upload: i64,
    /// Sort key "By episode number" / "By chapter number".
    pub number: f64,
    pub state: DownloadState,
    /// Pages finished (manga). Anime rows leave it at 0.
    pub pages_done: u32,
    /// Pages in the list (manga). `None` before the list is known, and always `None` for anime.
    pub pages_total: Option<u32>,
    /// Anime: percent 0..=100. Manga: the sum of page progress, each page at most 100.
    pub progress: i64,
    /// The source cannot be reached: the page shows "Waiting for source" and the unit does not run.
    pub waiting: bool,
}

/// One section: a source and its rows in queue order.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub source_id: i64,
    pub source_name: String,
    pub items: Vec<QueueItem>,
}

/// The queue as the page sees it. `flat` is the order the engine holds.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct QueueModel {
    flat: Vec<QueueItem>,
}

impl QueueModel {
    pub fn from_flat(flat: Vec<QueueItem>) -> Self {
        Self { flat }
    }

    /// The rows of one lane, in queue order. `units` is the lane's items (`Snapshot::of(kind)` once the snapshot type
    /// exists). Pages come from the live progress; `progress` is the percent for both kinds.
    pub fn from_units<'a>(units: impl IntoIterator<Item = &'a Arc<QueuedUnit>>) -> Self {
        let flat = units
            .into_iter()
            .map(|d| {
                let pages = d.progress.pages();
                QueueItem {
                    kind: d.kind,
                    item_id: d.unit.id,
                    parent_id: d.entry.id,
                    source_id: d.source.id,
                    source_name: d.source.display_name(),
                    title: d.entry.title.clone(),
                    item_name: d.unit.name.clone(),
                    date_upload: d.unit.date_upload,
                    number: d.unit.number,
                    state: d.state,
                    pages_done: pages.map_or(0, |(done, _)| done),
                    pages_total: pages.map(|(_, total)| total),
                    progress: i64::from(d.progress.percent()),
                    waiting: d.waiting,
                }
            })
            .collect();
        Self { flat }
    }

    pub fn flat(&self) -> &[QueueItem] {
        &self.flat
    }

    /// Groups by source id in first-appearance order. Two sources with the same name stay two sections (E-37).
    pub fn sections(&self) -> Vec<Section> {
        let mut out: Vec<Section> = Vec::new();
        for item in &self.flat {
            match out.iter_mut().find(|s| s.source_id == item.source_id) {
                Some(section) => section.items.push(item.clone()),
                None => out.push(Section {
                    source_id: item.source_id,
                    source_name: item.source_name.clone(),
                    items: vec![item.clone()],
                }),
            }
        }
        out
    }

    fn set_sections(&mut self, sections: Vec<Section>) {
        self.flat = sections.into_iter().flat_map(|s| s.items).collect();
    }

    /// Moves the item to the top of its own section. Returns `false` when the id is not queued.
    pub fn move_to_top(&mut self, item_id: i64) -> bool {
        self.move_in_section(item_id, Place::Top)
    }

    /// Moves the item to the bottom of its own section. Returns `false` when the id is not queued.
    pub fn move_to_bottom(&mut self, item_id: i64) -> bool {
        self.move_in_section(item_id, Place::Bottom)
    }

    fn move_in_section(&mut self, item_id: i64, place: Place) -> bool {
        let mut sections = self.sections();
        let Some(section) = sections.iter_mut().find(|s| s.items.iter().any(|i| i.item_id == item_id)) else {
            return false;
        };
        let Some(index) = section.items.iter().position(|i| i.item_id == item_id) else { return false };
        let item = section.items.remove(index);
        match place {
            Place::Top => section.items.insert(0, item),
            Place::Bottom => section.items.push(item),
        }
        self.set_sections(sections);
        true
    }
}

#[derive(Clone, Copy)]
enum Place {
    Top,
    Bottom,
}

/// The order the engine receives for a `reorder` call (E-15). Unknown ids are dropped, repeats keep their first place,
/// and live ids that were not listed follow in their current order. `None` when nothing has to be sent: an empty request
/// never clears the queue, and a result equal to `current` (a drop at the original position) is no reorder (T-25a-028).
pub fn normalize_order(current: &[i64], requested: &[i64]) -> Option<Vec<i64>> {
    if requested.is_empty() {
        return None;
    }
    let mut out: Vec<i64> = Vec::with_capacity(current.len());
    for id in requested {
        if current.contains(id) && !out.contains(id) {
            out.push(*id);
        }
    }
    for id in current {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    (out != current).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(item_id: i64, parent_id: i64, source_id: i64, date: i64, number: f64) -> QueueItem {
        QueueItem {
            kind: Kind::Anime,
            item_id,
            parent_id,
            source_id,
            source_name: format!("Source {source_id}"),
            title: format!("Title {parent_id}"),
            item_name: format!("Episode {item_id}"),
            date_upload: date,
            number,
            state: DownloadState::Queue,
            pages_done: 0,
            pages_total: None,
            progress: 0,
            waiting: false,
        }
    }

    fn ids(model: &QueueModel) -> Vec<i64> {
        model.flat().iter().map(|i| i.item_id).collect()
    }

    #[test]
    fn sections_follow_first_appearance_not_source_id() {
        // Source 9 appears first, so it is the first section even though its id is larger.
        let model = QueueModel::from_flat(vec![
            item(1, 10, 9, 0, 1.0),
            item(2, 20, 3, 0, 1.0),
            item(3, 10, 9, 0, 2.0),
        ]);
        let sections = model.sections();
        assert_eq!(sections.iter().map(|s| s.source_id).collect::<Vec<_>>(), vec![9, 3]);
        assert_eq!(sections[0].items.iter().map(|i| i.item_id).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn two_sources_with_the_same_name_are_two_sections() {
        let mut second = item(2, 20, 4, 0, 1.0);
        second.source_name = "Source 3".to_owned();
        let mut first = item(1, 10, 3, 0, 1.0);
        first.source_name = "Source 3".to_owned();
        let model = QueueModel::from_flat(vec![first, second]);
        assert_eq!(model.sections().len(), 2, "E-37: the id decides the section, not the name");
    }

    #[test]
    fn flat_order_across_sections_is_kept_on_a_drop() {
        let flat = vec![item(1, 10, 1, 0, 1.0), item(2, 20, 2, 0, 1.0), item(3, 10, 1, 0, 2.0)];
        let model = QueueModel::from_flat(flat.clone());
        assert_eq!(model.flat(), flat.as_slice(), "T-E34: the model keeps the order it was given");
    }

    #[test]
    fn move_to_top_and_bottom_act_inside_the_items_own_section() {
        let mut model = QueueModel::from_flat(vec![
            item(1, 10, 1, 0, 1.0),
            item(2, 20, 2, 0, 1.0),
            item(3, 10, 1, 0, 1.0),
            item(4, 10, 1, 0, 1.0),
        ]);
        // The flat list is rebuilt section by section, as Kotlin rebuilds it from the headers.
        assert!(model.move_to_top(3));
        assert_eq!(ids(&model), vec![3, 1, 4, 2]);
        assert!(model.move_to_bottom(3));
        assert_eq!(ids(&model), vec![1, 4, 3, 2]);
        assert!(!model.move_to_top(99), "unknown id is a no-op");
    }

    #[test]
    fn reorder_ids_drops_unknown_keeps_unlisted_and_ignores_empty() {
        assert_eq!(normalize_order(&[1, 2, 3], &[]), None, "an empty list never clears the queue");
        assert_eq!(normalize_order(&[1, 2, 3], &[3, 99, 1]), Some(vec![3, 1, 2]), "E-15");
        assert_eq!(normalize_order(&[1, 2, 3], &[2, 2]), Some(vec![2, 1, 3]), "repeats keep their first place");
        assert_eq!(normalize_order(&[1, 2, 3], &[1, 2, 3]), None, "the same order is no reorder");
        assert_eq!(normalize_order(&[1, 2, 3], &[1, 99]), None, "unknown ids and the rest keep the order");
    }

    #[test]
    fn from_units_maps_both_kinds_and_the_waiting_flag() {
        use crate::unit::fixtures::queued;
        let manga = queued(Kind::Manga, 1, 10, 5);
        manga.progress.set_pages(3, 12);
        manga.progress.set_percent(25);
        let mut anime = queued(Kind::Anime, 2, 20, 6);
        anime.waiting = true;
        let units = vec![Arc::new(manga), Arc::new(anime)];
        let model = QueueModel::from_units(&units);
        let rows = model.flat();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].kind, rows[0].item_id, rows[0].parent_id), (Kind::Manga, 10, 5));
        assert_eq!((rows[0].pages_done, rows[0].pages_total, rows[0].progress), (3, Some(12), 25));
        assert_eq!(rows[0].source_name, "Source 1 (en)");
        assert!(!rows[0].waiting);
        assert_eq!((rows[1].kind, rows[1].pages_total, rows[1].state), (Kind::Anime, None, DownloadState::Queue));
        assert!(rows[1].waiting);
    }
}
