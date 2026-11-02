//! Allowed values of integer settings, read by the settings widgets and by consumers that need a
//! valid value (T-02a-036). Import keeps the stored value as it is; a consumer that needs a valid
//! one calls [`Constraint::clamped`].

/// The values an integer setting may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    /// Every whole number from `min` to `max`, both included.
    Range { min: i64, max: i64 },
    /// The listed values only, in the order the widget shows them.
    Choices(&'static [i64]),
}

impl Constraint {
    pub fn contains(&self, value: i64) -> bool {
        match *self {
            Constraint::Range { min, max } => (min..=max).contains(&value),
            Constraint::Choices(choices) => choices.contains(&value),
        }
    }

    /// `(min, max)` of a range, `None` for a list of choices. For a slider's two ends.
    pub const fn bounds(&self) -> Option<(i32, i32)> {
        match *self {
            Constraint::Range { min, max } => Some((min as i32, max as i32)),
            Constraint::Choices(_) => None,
        }
    }

    /// Every allowed value, in the order a widget lists them.
    pub fn values(&self) -> Vec<i64> {
        match *self {
            Constraint::Range { min, max } => (min..=max).collect(),
            Constraint::Choices(choices) => choices.to_vec(),
        }
    }

    /// The nearest allowed value. A value between two choices goes to the lower one. A range clamps.
    pub fn clamped(&self, value: i64) -> i64 {
        match *self {
            Constraint::Range { min, max } => value.clamp(min, max),
            Constraint::Choices(choices) => choices
                .iter()
                .copied()
                .min_by_key(|c| ((c - value).abs(), *c))
                .unwrap_or(value),
        }
    }
}

pub const DOWNLOAD_SLOTS: Constraint = Constraint::Range { min: 1, max: 5 };
pub const REMOVE_AFTER_READ_SLOTS: Constraint = Constraint::Choices(&[-1, 0, 1, 2, 3, 4]);
pub const BACKUP_INTERVAL: Constraint = Constraint::Choices(&[0, 6, 12, 24, 48, 168]);
/// The cache size limit in megabytes.
pub const CACHE_SIZE_MB: Constraint = Constraint::Choices(&[100, 250, 500, 1000, 2000, 5000]);

/// Registry key of every integer setting with a constraint.
const CONSTRAINED: &[(&str, Constraint)] = &[
    ("download_slots", DOWNLOAD_SLOTS),
    ("remove_after_read_slots", REMOVE_AFTER_READ_SLOTS),
    ("backup_interval", BACKUP_INTERVAL),
    ("cache_size_mb", CACHE_SIZE_MB),
];

/// The constraint of `key`, or `None` when the key has none.
pub fn constraint(key: &str) -> Option<Constraint> {
    CONSTRAINED.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

/// Every constrained key with its constraint, in table order.
pub fn constrained_keys() -> &'static [(&'static str, Constraint)] {
    CONSTRAINED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;
    use crate::values::PrefValue;

    #[test]
    fn range_contains_and_clamps_both_ends() {
        assert!(DOWNLOAD_SLOTS.contains(1) && DOWNLOAD_SLOTS.contains(5));
        assert!(!DOWNLOAD_SLOTS.contains(0) && !DOWNLOAD_SLOTS.contains(6));
        assert_eq!(DOWNLOAD_SLOTS.clamped(0), 1);
        assert_eq!(DOWNLOAD_SLOTS.clamped(9), 5);
    }

    #[test]
    fn choices_clamp_to_the_nearest_value_and_ties_go_low() {
        assert_eq!(REMOVE_AFTER_READ_SLOTS.clamped(-7), -1);
        assert_eq!(BACKUP_INTERVAL.clamped(168), 168);
    }

    #[test]
    fn every_constrained_key_is_a_registry_int_and_its_default_is_allowed() {
        for (key, constraint) in constrained_keys() {
            let (meta, _) = registry::lookup(key).unwrap_or_else(|| panic!("{key} is not in the registry"));
            assert_eq!(meta.kind, registry::Kind::Int, "{key}");
            let PrefValue::Int(default) = meta.default.value() else {
                panic!("{key} default is not an int");
            };
            assert!(constraint.contains(i64::from(default)), "{key} default {default} is outside its constraint");
        }
    }

    #[test]
    fn bounds_give_the_slider_ends_of_a_range_only() {
        assert_eq!(BACKUP_INTERVAL.bounds(), None);
    }

    #[test]
    fn values_list_a_range_in_full_and_the_choices_in_order() {
        assert_eq!(DOWNLOAD_SLOTS.values(), [1, 2, 3, 4, 5]);
        assert_eq!(BACKUP_INTERVAL.values(), [0, 6, 12, 24, 48, 168]);
    }

    #[test]
    fn constraint_lookup_finds_the_listed_keys_only() {
        assert_eq!(constraint("download_slots"), Some(DOWNLOAD_SLOTS));
        assert_eq!(constraint("download_speed_limit"), None);
    }
}
