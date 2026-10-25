//! Seek primitives (area 45 `PlayerViewModel` seek functions, behaviour 8.2; T-45b-023). Pure: the
//! caller writes the returned commands to mpv. Times are whole seconds.

/// Flag for a relative seek. `precise` is `pref_player_smooth_seek`.
pub fn relative_flag(precise: bool) -> &'static str {
    if precise {
        "relative+exact"
    } else {
        "relative"
    }
}

/// Flag for an absolute seek. `precise = false` uses keyframes.
pub fn absolute_flag(precise: bool) -> &'static str {
    if precise {
        "absolute"
    } else {
        "absolute+keyframes"
    }
}

/// A seek to an absolute position, waiting until the duration is known (E-11).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seeker {
    pending: Option<(i64, bool)>,
}

impl Seeker {
    /// `seekTo`. Returns the command `(position, flag)` to send. An unknown duration (zero) queues
    /// the seek, and a position outside `0..=duration` is ignored.
    pub fn seek_to(
        &mut self,
        position: i64,
        precise: bool,
        duration: i64,
    ) -> Option<(i64, &'static str)> {
        if duration <= 0 {
            self.pending = Some((position, precise));
            return None;
        }
        if !(0..=duration).contains(&position) {
            return None;
        }
        Some((position, absolute_flag(precise)))
    }

    /// Runs the queued seek once the duration is known.
    pub fn duration_known(&mut self, duration: i64) -> Option<(i64, &'static str)> {
        let (position, precise) = self.pending.take()?;
        self.seek_to(position, precise, duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_strings_match_the_spec() {
        assert_eq!(relative_flag(false), "relative");
        assert_eq!(relative_flag(true), "relative+exact");
        assert_eq!(absolute_flag(true), "absolute");
        assert_eq!(absolute_flag(false), "absolute+keyframes");
    }

    #[test]
    fn t43_seek_to_out_of_range_is_ignored() {
        let mut s = Seeker::default();
        assert_eq!(s.seek_to(-1, true, 100), None);
        assert_eq!(s.seek_to(101, true, 100), None);
        assert_eq!(s.seek_to(100, true, 100), Some((100, "absolute")));
        assert_eq!(s.seek_to(0, false, 100), Some((0, "absolute+keyframes")));
    }

    #[test]
    fn seek_to_waits_for_the_duration() {
        let mut s = Seeker::default();
        assert_eq!(s.seek_to(50, true, 0), None);
        assert_eq!(s.duration_known(100), Some((50, "absolute")));
        assert_eq!(s.duration_known(100), None);
    }
}
