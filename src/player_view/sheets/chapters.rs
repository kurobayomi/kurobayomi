//! The chapter list of the player's menu, from mpv's chapter list of the playing file (only when it has chapters). An
//! entry seeks to the chapter's start and plays on. Entry text comes from `kuro_player::chapters`.

use std::rc::Rc;

use kuro_player::chapters::{row_label, row_time};

use super::super::Shared;
use super::Choice;

/// The chapters of the playing file, the playing one marked; empty when the file has none. Choosing one plays from its
/// start.
pub fn choices(shared: &Shared) -> Vec<Choice> {
    let playing = shared.borrow().playback.mpv().get_i64("chapter");
    (0..count(shared))
        .map(|index| {
            let (name, start) = chapter(shared, index);
            let shared = Rc::clone(shared);
            Choice {
                title: row_label(index as usize, &name),
                subtitle: Some(row_time(start as f32)),
                current: playing == Some(index),
                enabled: true,
                pick: Box::new(move || go_to(&shared, index)),
            }
        })
        .collect()
}

fn count(shared: &Shared) -> i64 {
    shared.borrow().playback.mpv().get_i64("chapter-list/count").unwrap_or(0)
}

/// The name and start (in seconds) of chapter `index`. A missing name is empty.
fn chapter(shared: &Shared, index: i64) -> (String, f64) {
    let player = shared.borrow();
    let mpv = player.playback.mpv();
    (
        mpv.get_string(&format!("chapter-list/{index}/title")).unwrap_or_default(),
        mpv.get_f64(&format!("chapter-list/{index}/time")).unwrap_or(0.0),
    )
}

/// Jumps to chapter `index` and plays on.
fn go_to(shared: &Shared, index: i64) {
    let player = shared.borrow();
    if let Err(e) = player.playback.mpv().set_property_i64("chapter", index) {
        tracing::warn!(error = %e, "player: the chapter was not selected");
    }
    if let Err(e) = player.playback.mpv().set_property_flag("pause", false) {
        tracing::warn!(error = %e, "player: the pause state was not cleared");
    }
}
