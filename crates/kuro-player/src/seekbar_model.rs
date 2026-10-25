//! Seekbar model: chapter ticks, displayed value, labels and drag rules (area 46 `SeekBar.kt` and
//! `PlayerControls.kt`, T-46a-010, T-46a-011). Pure: the GTK widget draws the fractions and the
//! labels returned here. Times are seconds. A duration of zero means unknown.

use crate::time::{pretty_time, pretty_time_signed};

/// Tick positions in seconds: one per chapter start within `0..=duration`, each once. A synthetic
/// `0` is prepended when the first chapter does not start at zero. An unknown duration gives none.
pub fn seek_segments(chapter_starts: &[f32], duration: f32) -> Vec<f32> {
    if duration <= 0.0 {
        return Vec::new();
    }
    let mut ticks: Vec<f32> = Vec::new();
    for &start in chapter_starts
        .iter()
        .filter(|s| (0.0..=duration).contains(*s))
    {
        if !ticks.contains(&start) {
            ticks.push(start);
        }
    }
    if ticks.first().is_some_and(|first| *first != 0.0) {
        ticks.insert(0, 0.0);
    }
    ticks
}

/// The value the bar shows: gesture seek first, then the drag, then the playhead (E-46-036).
pub fn displayed_value(gesture: Option<f32>, drag: Option<f32>, playhead: f32) -> f32 {
    gesture.or(drag).unwrap_or(playhead)
}

/// The thumb fraction. Zero when the duration is unknown.
pub fn fraction(value: f32, duration: f32) -> f32 {
    if duration > 0.0 {
        (value / duration).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The read-ahead fill: the thumb fraction plus the buffered seconds from `demuxer-cache-time`
/// (the absolute end of the cache, as mpv reports it) over the duration. Never behind the
/// playhead, and zero while the duration or the cache time is unknown.
pub fn read_ahead_fraction(demuxer_cache_time: Option<f32>, playhead: f32, duration: f32) -> f32 {
    match demuxer_cache_time {
        Some(cache_end) if duration > 0.0 => fraction(cache_end.max(playhead), duration),
        _ => 0.0,
    }
}

/// Drags are ignored while the duration is unknown (E-46-015).
pub fn drags_enabled(duration: f32) -> bool {
    duration > 0.0
}

/// Width of the position label in logical pixels (T-46a-011); the GTK label sets it as its width.
pub const POSITION_LABEL_WIDTH_PX: i32 = 92;

/// The position label, `POSITION_LABEL_WIDTH_PX` wide in the GTK layer.
pub fn position_label(position: f32) -> String {
    pretty_time(position as i64)
}

/// The duration label. Total time, or `position - duration` with a sign once inverted. Unknown
/// duration shows `--:--`.
pub fn duration_label(position: f32, duration: f32, inverted: bool) -> String {
    if duration <= 0.0 {
        "--:--".to_owned()
    } else if inverted {
        pretty_time_signed(position as i64 - duration as i64)
    } else {
        pretty_time(duration as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_once_each_with_a_synthetic_zero_when_needed() {
        assert_eq!(
            seek_segments(&[85.0, 1330.0], 1420.0),
            vec![0.0, 85.0, 1330.0]
        );
        assert_eq!(seek_segments(&[0.0, 85.0], 1420.0), vec![0.0, 85.0]);
        assert_eq!(seek_segments(&[85.0, 85.0], 1420.0), vec![0.0, 85.0]);
    }

    #[test]
    fn ticks_drop_chapters_outside_the_duration_and_keep_the_end() {
        assert_eq!(
            seek_segments(&[10.0, 1420.0, 1500.0], 1420.0),
            vec![0.0, 10.0, 1420.0]
        );
    }

    #[test]
    fn unknown_duration_has_no_ticks_and_ignores_drags() {
        assert!(seek_segments(&[10.0], 0.0).is_empty());
        assert_eq!(fraction(10.0, 0.0), 0.0);
        assert!(!drags_enabled(0.0));
    }

    #[test]
    fn displayed_value_prefers_gesture_then_drag_then_playhead() {
        assert_eq!(displayed_value(Some(1.0), Some(2.0), 3.0), 1.0);
        assert_eq!(displayed_value(None, Some(2.0), 3.0), 2.0);
        assert_eq!(displayed_value(None, None, 3.0), 3.0);
    }

    #[test]
    fn read_ahead_is_the_cache_end_over_the_duration_never_behind_the_playhead() {
        assert_eq!(read_ahead_fraction(Some(710.0), 100.0, 1420.0), 0.5);
        assert_eq!(
            read_ahead_fraction(Some(50.0), 100.0, 1420.0),
            fraction(100.0, 1420.0)
        );
        assert_eq!(read_ahead_fraction(Some(5000.0), 0.0, 1420.0), 1.0);
        assert_eq!(read_ahead_fraction(None, 100.0, 1420.0), 0.0);
        assert_eq!(read_ahead_fraction(Some(10.0), 0.0, 0.0), 0.0);
    }

    #[test]
    fn duration_label_shows_the_total_or_unknown() {
        assert_eq!(duration_label(85.0, 1420.0, false), "23:40");
        assert_eq!(duration_label(0.0, 0.0, false), "--:--");
        assert_eq!(position_label(85.0), "01:25");
    }
}
