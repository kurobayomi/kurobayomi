//! Tap zones (T-44a-008, T-44a-009, T-44a-010, T-44a-020, T-44a-085, B§2). Coordinates are normalised to the viewer
//! (0 to 1). A region is half-open (`left <= x < right`, `top <= y < bottom`), and a point in no region is `Menu`.

use kuro_prefs::reader::TappingInvertMode;

use crate::reader::viewers::ViewerKind;
use crate::reader::viewers::pager::nav::PageCmd;

/// `RectF(left, top, right, bottom)` in normalised coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self { left, top, right, bottom }
    }

    /// Half-open containment (`RectF.contains`).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.left <= x && x < self.right && self.top <= y && y < self.bottom
    }

    /// `RectFExtensions`: a horizontal inversion mirrors x, a vertical one mirrors y, both mirror both.
    pub fn invert(self, mode: TappingInvertMode) -> Self {
        let (left, top, right, bottom) = (self.left, self.top, self.right, self.bottom);
        let (h, v) = (mode.should_invert_horizontal(), mode.should_invert_vertical());
        let (left, right) = if h { (1.0 - right, 1.0 - left) } else { (left, right) };
        let (top, bottom) = if v { (1.0 - bottom, 1.0 - top) } else { (top, bottom) };
        Self { left, top, right, bottom }
    }
}

/// What a tap does, before the viewer maps it to a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Menu,
    Prev,
    Next,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub action: Action,
}

/// `ViewerNavigation` mode. `Default` resolves per viewer; unknown stored codes fall back to it (D§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavMode {
    Default,
    LShaped,
    KindleIsh,
    Edge,
    RightAndLeft,
    Disabled,
}

impl NavMode {
    pub fn from_stored(code: i32) -> Self {
        match code {
            1 => Self::LShaped,
            2 => Self::KindleIsh,
            3 => Self::Edge,
            4 => Self::RightAndLeft,
            5 => Self::Disabled,
            _ => Self::Default,
        }
    }
}

const PREV: Action = Action::Prev;
const NEXT: Action = Action::Next;

/// Mode 1, `LNavigation` (B§2.3).
pub const L_SHAPED: &[Region] = &[
    Region { rect: Rect::new(0.00, 0.33, 0.33, 0.66), action: PREV },
    Region { rect: Rect::new(0.00, 0.00, 1.00, 0.33), action: PREV },
    Region { rect: Rect::new(0.66, 0.33, 1.00, 0.66), action: NEXT },
    Region { rect: Rect::new(0.00, 0.66, 1.00, 1.00), action: NEXT },
];

/// Mode 2, `KindlishNavigation`.
pub const KINDLE_ISH: &[Region] = &[
    Region { rect: Rect::new(0.33, 0.33, 1.00, 1.00), action: NEXT },
    Region { rect: Rect::new(0.00, 0.33, 0.33, 1.00), action: PREV },
];

/// Mode 3, `EdgeNavigation`.
pub const EDGE: &[Region] = &[
    Region { rect: Rect::new(0.00, 0.00, 0.33, 1.00), action: NEXT },
    Region { rect: Rect::new(0.33, 0.66, 0.66, 1.00), action: PREV },
    Region { rect: Rect::new(0.66, 0.00, 1.00, 1.00), action: NEXT },
];

/// Mode 4, `RightAndLeftNavigation`. Absolute left and right, not reading-direction aware.
pub const RIGHT_AND_LEFT: &[Region] = &[
    Region { rect: Rect::new(0.00, 0.00, 0.33, 1.00), action: Action::Left },
    Region { rect: Rect::new(0.66, 0.00, 1.00, 1.00), action: Action::Right },
];

/// Mode 5, `DisabledNavigation`: every tap is `Menu`.
pub const DISABLED: &[Region] = &[];

/// The regions a mode uses for a viewer. `Default` is `RightAndLeft` for the horizontal pager and `L` otherwise.
pub fn layout(mode: NavMode, viewer: ViewerKind) -> &'static [Region] {
    let default = match viewer {
        ViewerKind::LeftToRight | ViewerKind::RightToLeft => RIGHT_AND_LEFT,
        ViewerKind::Vertical | ViewerKind::Webtoon | ViewerKind::WebtoonGapped => L_SHAPED,
    };
    match mode {
        NavMode::Default => default,
        NavMode::LShaped => L_SHAPED,
        NavMode::KindleIsh => KINDLE_ISH,
        NavMode::Edge => EDGE,
        NavMode::RightAndLeft => RIGHT_AND_LEFT,
        NavMode::Disabled => DISABLED,
    }
}

/// The action at normalised point `(x, y)`, with the regions inverted by `invert` first (B§2.2).
pub fn hit_test(regions: &[Region], invert: TappingInvertMode, x: f64, y: f64) -> Action {
    regions.iter().find(|r| r.rect.invert(invert).contains(x, y)).map_or(Action::Menu, |r| r.action)
}

/// What a tap means for the pager (B§2.4). `Right` and `Left` are absolute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapOutcome {
    ToggleMenu,
    Page(PageCmd),
    ScrollDown,
    ScrollUp,
}

pub fn pager_tap(action: Action) -> TapOutcome {
    match action {
        Action::Menu => TapOutcome::ToggleMenu,
        Action::Next => TapOutcome::Page(PageCmd::Next),
        Action::Prev => TapOutcome::Page(PageCmd::Prev),
        Action::Right => TapOutcome::Page(PageCmd::Right),
        Action::Left => TapOutcome::Page(PageCmd::Left),
    }
}

/// The webtoon has no reading direction, so next and right both scroll down (B§2.4).
pub fn webtoon_tap(action: Action) -> TapOutcome {
    match action {
        Action::Menu => TapOutcome::ToggleMenu,
        Action::Next | Action::Right => TapOutcome::ScrollDown,
        Action::Prev | Action::Left => TapOutcome::ScrollUp,
    }
}

/// Overlay colours as ARGB (B§2.5, D§1.7). The overlay in the reader core reads these.
pub mod colors {
    pub const MENU: u32 = 0xCC95_818D;
    pub const PREV: u32 = 0xCCFF_7733;
    pub const NEXT: u32 = 0xCC84_E296;
    pub const LEFT: u32 = 0xCC7D_1128;
    pub const RIGHT: u32 = 0xCCA6_CFD5;

    pub fn of(action: super::Action) -> u32 {
        use super::Action::*;
        match action {
            Menu => MENU,
            Prev => PREV,
            Next => NEXT,
            Left => LEFT,
            Right => RIGHT,
        }
    }
}

/// The string resource key and the English text of each overlay label (B§14). Missing translations fall back to
/// the English text.
pub fn label(action: Action) -> (&'static str, &'static str) {
    match action {
        Action::Menu => ("action_menu", "Menu"),
        Action::Prev => ("nav_zone_prev", "Prev"),
        Action::Next => ("nav_zone_next", "Next"),
        Action::Left => ("nav_zone_left", "Left"),
        Action::Right => ("nav_zone_right", "Right"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TappingInvertMode::*;

    fn at(mode: NavMode, viewer: ViewerKind, x: f64, y: f64) -> Action {
        hit_test(layout(mode, viewer), NONE, x, y)
    }

    #[test]
    fn the_region_tables_match_the_observed_values_and_order() {
        assert_eq!(L_SHAPED.len(), 4);
        assert_eq!(L_SHAPED[0], Region { rect: Rect::new(0.0, 0.33, 0.33, 0.66), action: Action::Prev });
    }

    #[test]
    fn default_is_right_and_left_for_the_horizontal_pager_and_l_otherwise() {
        assert_eq!(layout(NavMode::Default, ViewerKind::LeftToRight), RIGHT_AND_LEFT);
        assert_eq!(layout(NavMode::Default, ViewerKind::RightToLeft), RIGHT_AND_LEFT);
        assert_eq!(layout(NavMode::Default, ViewerKind::Vertical), L_SHAPED);
        assert_eq!(layout(NavMode::Default, ViewerKind::Webtoon), L_SHAPED);
    }

    #[test]
    fn l_shaped_leaves_the_middle_block_to_the_menu() {
        assert_eq!(at(NavMode::LShaped, ViewerKind::Webtoon, 0.5, 0.5), Action::Menu);
        assert_eq!(at(NavMode::LShaped, ViewerKind::Webtoon, 0.1, 0.5), Action::Prev);
        assert_eq!(at(NavMode::LShaped, ViewerKind::Webtoon, 0.9, 0.5), Action::Next);
    }

    #[test]
    fn pager_and_webtoon_dispatch_differ_as_the_observed_code_does() {
        assert_eq!(pager_tap(Action::Right), TapOutcome::Page(PageCmd::Right));
        assert_eq!(webtoon_tap(Action::Right), TapOutcome::ScrollDown);
        assert_eq!(webtoon_tap(Action::Left), TapOutcome::ScrollUp);
        assert_eq!(pager_tap(Action::Menu), TapOutcome::ToggleMenu);
    }

    #[test]
    fn overlay_colours_and_labels_are_the_observed_constants() {
        assert_eq!(colors::of(Action::Menu), 0xCC95818D);
        assert_eq!(colors::of(Action::Right), 0xCCA6CFD5);
        assert_eq!(label(Action::Prev), ("nav_zone_prev", "Prev"));
    }

    const GRID: [f64; 12] = [0.0, 0.1, 0.2, 0.33, 0.4, 0.5, 0.6, 0.66, 0.7, 0.8, 0.9, 1.0];

    fn letter(action: Action) -> char {
        match action {
            Action::Menu => 'M',
            Action::Prev => 'P',
            Action::Next => 'N',
            Action::Left => 'L',
            Action::Right => 'R',
        }
    }

    /// One layout as a golden value: a row of letters per `y` and a column per `x`.
    fn golden_of(mode: NavMode, viewer: ViewerKind) -> serde_json::Value {
        let regions = layout(mode, viewer);
        let rows: Vec<String> = GRID.iter().map(|y| GRID.iter().map(|x| letter(hit_test(regions, NONE, *x, *y))).collect()).collect();
        serde_json::json!({
            "about": "hit_test outcome per normalised point: M menu, P prev, N next, L left, R right; rows are y, columns are x",
            "xs": GRID,
            "ys": GRID,
            "rows": rows,
        })
    }

    /// T-44a-078: the default layout, which the app always uses (the tap-zone setting is cut), from the golden files.
    /// It is one file per viewer family, since it resolves differently for each (B§2.3).
    #[test]
    fn the_golden_navigation_cases_hold_for_the_default_layouts() {
        use crate::reader::golden::check;
        check("nav_default_pager.json", &golden_of(NavMode::Default, ViewerKind::LeftToRight));
        check("nav_default_webtoon.json", &golden_of(NavMode::Default, ViewerKind::Webtoon));
    }
}
