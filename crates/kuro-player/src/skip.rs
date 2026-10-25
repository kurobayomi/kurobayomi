//! Skip-intro state machine (area 45 `PlayerViewModel.setChapter`, `onSkipIntro`,
//! `showSkipIntroButton`, `updateSkipIntroButton`; behaviour 6.8, T-45b-022). Pure: the caller
//! passes the chapter list and the position, and applies the returned label, toast and seek.
//!
//! Two fixes to the Kotlin: the countdown resets whenever the current chapter changes (E-09), and
//! only the whole-second tick decrements it, never the extra sync calls (E-10).

use crate::chapters::{Segment, current_chapter, skip_label};
use crate::errors::{
    PlayerMessage, aniskip_countdown_toast, aniskip_skip_text, intro_skipped_text,
};
use crate::types::ChapterType;

/// The preferences the machine reads once (`introSkipEnabled`, `autoSkip`, `netflixStyle`,
/// `defaultWaitingTime`). The defaults are the preference defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkipConfig {
    pub enabled: bool,
    pub auto_skip: bool,
    pub netflix: bool,
    pub default_wait: i32,
}

impl Default for SkipConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_skip: false,
            netflix: false,
            default_wait: 5,
        }
    }
}

/// A seek the machine asks for, with the text shown after it (`seekToWithText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipSeek {
    /// Whole seconds (`nextChapterPos.toInt()`).
    pub position: i64,
    pub text: String,
}

/// What to do with the skip button label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label {
    /// Leave `skip_intro_text` as it is.
    Keep,
    /// Set `skip_intro_text`. `None` hides the button.
    Set(Option<String>),
}

/// The result of one tick or sync call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tick {
    pub label: Label,
    /// The countdown toast, on the first call of a countdown only.
    pub toast: Option<String>,
    pub seek: Option<SkipSeek>,
    /// The countdown value this call read (netflix style); `None` outside a countdown.
    pub waiting: Option<i32>,
}

impl Tick {
    const NONE: Self = Self {
        label: Label::Keep,
        toast: None,
        seek: None,
        waiting: None,
    };
}

/// The result of tapping the skip button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// There is no current chapter.
    Nothing,
    /// The countdown was cancelled. The button shows its normal label on the next tick.
    Cancelled,
    Seek(SkipSeek),
}

/// The machine. `waiting` is `waitingSkipIntro`; `-1` means the user cancelled the countdown.
#[derive(Clone, Debug)]
pub struct SkipIntro {
    config: SkipConfig,
    waiting: i32,
    chapter: Option<Segment>,
}

impl SkipIntro {
    pub fn new(config: SkipConfig) -> Self {
        Self {
            config,
            waiting: config.default_wait,
            chapter: None,
        }
    }

    /// The countdown value now.
    pub fn waiting(&self) -> i32 {
        self.waiting
    }

    /// `setChapter` on a whole-second tick: decrements the netflix countdown.
    pub fn on_tick(&mut self, chapters: &[Segment], pos: f32) -> Tick {
        self.step(chapters, pos, true)
    }

    /// `setChapter` from the sync points (after a chapter merge, after the first load). It sets the
    /// label, but it neither counts down, toasts nor seeks (E-10).
    pub fn sync(&mut self, chapters: &[Segment], pos: f32) -> Tick {
        self.step(chapters, pos, false)
    }

    fn step(&mut self, chapters: &[Segment], pos: f32, ticking: bool) -> Tick {
        let Some(index) = current_chapter(chapters, pos) else {
            return Tick::NONE;
        };
        let chapter = &chapters[index];
        if self.chapter.as_ref() != Some(chapter) {
            // E-09: a new chapter starts its own countdown.
            self.chapter = Some(chapter.clone());
            self.waiting = self.config.default_wait;
        }
        if !self.config.enabled {
            return Tick::NONE;
        }
        if chapter.kind == ChapterType::Other {
            self.waiting = self.config.default_wait;
            return Tick {
                label: Label::Set(None),
                ..Tick::NONE
            };
        }
        let next_start = chapters.get(index + 1).map_or(pos, |c| c.start);
        let seek = |text: String| SkipSeek {
            position: next_start as i64,
            text,
        };
        let manual = || Label::Set(skip_label(chapter.kind));

        if self.config.netflix {
            let waiting = self.waiting;
            let mut tick = Tick {
                waiting: Some(waiting),
                ..Tick::NONE
            };
            if ticking && waiting == self.config.default_wait {
                tick.toast = Some(aniskip_countdown_toast(&chapter.name, i64::from(waiting)));
            }
            if waiting > 0 {
                tick.label = Label::Set(Some(PlayerMessage::AniskipDontSkip.text()));
            } else if waiting == 0 {
                if ticking {
                    tick.seek = Some(seek(aniskip_skip_text(&chapter.name)));
                }
            } else {
                tick.label = manual();
            }
            if ticking {
                self.waiting -= 1;
            }
            tick
        } else if self.config.auto_skip {
            Tick {
                seek: ticking.then(|| seek(intro_skipped_text(&chapter.name))),
                ..Tick::NONE
            }
        } else {
            Tick {
                label: manual(),
                ..Tick::NONE
            }
        }
    }

    /// `onSkipIntro`: the user taps the button. During a netflix countdown it cancels it; otherwise
    /// it seeks to the start of the next chapter.
    pub fn on_skip(&mut self, chapters: &[Segment], pos: f32) -> Tap {
        let Some(index) = current_chapter(chapters, pos) else {
            return Tap::Nothing;
        };
        if self.waiting > 0 && self.config.netflix {
            self.waiting = -1;
            return Tap::Cancelled;
        }
        let next_start = chapters.get(index + 1).map_or(pos, |c| c.start);
        Tap::Seek(SkipSeek {
            position: next_start as i64,
            text: aniskip_skip_text(&chapters[index].name),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(index: i32, name: &str, start: f32, kind: ChapterType) -> Segment {
        Segment {
            index,
            name: name.to_owned(),
            start,
            kind,
        }
    }

    /// `[Other 0, Opening 10, Other 100, Ending 200, Other 300]`.
    fn chapters() -> Vec<Segment> {
        vec![
            seg(-1, "", 0.0, ChapterType::Other),
            seg(-2, "Opening", 10.0, ChapterType::Opening),
            seg(-1, "", 100.0, ChapterType::Other),
            seg(-2, "Ending", 200.0, ChapterType::Ending),
            seg(-1, "", 300.0, ChapterType::Other),
        ]
    }

    /// T33: without netflix style and auto skip the label is the type's skip text.
    #[test]
    fn t33_manual_mode_shows_the_skip_label() {
        let mut machine = SkipIntro::new(SkipConfig::default());
        let list = chapters();
        let tick = machine.on_tick(&list, 50.0);
        assert_eq!(tick.label, Label::Set(skip_label(ChapterType::Opening)));
        assert!(
            skip_label(ChapterType::Opening)
                .unwrap()
                .contains("Opening")
        );
        assert_eq!(tick.seek, None);
        assert_eq!(
            machine.on_skip(&list, 50.0),
            Tap::Seek(SkipSeek {
                position: 100,
                text: aniskip_skip_text("Opening")
            })
        );
    }

    #[test]
    fn no_chapter_does_nothing() {
        let mut machine = SkipIntro::new(SkipConfig::default());
        assert_eq!(machine.on_tick(&[], 50.0), Tick::NONE);
        assert_eq!(machine.on_skip(&[], 50.0), Tap::Nothing);
        // before the first chapter
        let late = [seg(-2, "Opening", 30.0, ChapterType::Opening)];
        assert_eq!(machine.on_tick(&late, 5.0), Tick::NONE);
    }

    #[test]
    fn the_last_chapter_skips_to_the_current_position() {
        let mut machine = SkipIntro::new(SkipConfig::default());
        let list = [seg(-2, "Ending", 10.0, ChapterType::Ending)];
        assert_eq!(
            machine.on_skip(&list, 42.7),
            Tap::Seek(SkipSeek {
                position: 42,
                text: aniskip_skip_text("Ending")
            })
        );
    }
}
