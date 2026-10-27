//! Drag to select (issue #7), for a cover grid or an item list. A press on the view's empty space (not on an item's own
//! press area) followed by a drag draws a translucent box, and every item on screen that the box touches is selected.
//!
//! The helper holds no selection of its own: it asks the page whether an id is selected and flips one through the page's
//! own toggle, the call a Ctrl+click makes, so the page's selection stays the only one (never GTK's selection model or its
//! rubberband). Ctrl held at the press keeps what was selected; otherwise the drag replaces it. An item the box leaves
//! again goes back to what it was before the drag. A press that does not move past `THRESHOLD` stays a plain click.
//!
//! Known limit: only the items on screen are hit; the view does not scroll while dragging.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, graphene};

/// Marks an item's own press area (`mark`): a drag never starts on it.
const ITEM_CLASS: &str = "kb-drag-item";
/// The widget name of a marked item that can be selected: the prefix and its id.
const NAME_PREFIX: &str = "pick:";
/// How far, in pixels, the pointer moves before a press becomes a drag.
const THRESHOLD: f64 = 8.0;

/// A box in the view's coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The box between two corners, in either direction.
    pub fn spanning(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x: x0.min(x1), y: y0.min(y1), width: (x1 - x0).abs(), height: (y1 - y0).abs() }
    }

    fn touches(&self, other: &Rect) -> bool {
        self.x <= other.x + other.width
            && other.x <= self.x + self.width
            && self.y <= other.y + other.height
            && other.y <= self.y + self.height
    }
}

/// The ids of the items whose box the drag box touches, in item order.
pub fn hits(band: Rect, items: &[(i64, Rect)]) -> Vec<i64> {
    items.iter().filter(|(_, rect)| band.touches(rect)).map(|(id, _)| *id).collect()
}

/// The ids to flip so that an item on screen is selected exactly when it was selected before the drag (`before`) or the
/// box touches it (`hit`). `now` is each item on screen with whether it is selected now.
pub fn flips(now: &[(i64, bool)], before: &HashMap<i64, bool>, hit: &[i64]) -> Vec<i64> {
    now.iter()
        .filter(|(id, selected)| {
            let wanted = before.get(id).copied().unwrap_or(false) || hit.contains(id);
            wanted != *selected
        })
        .map(|(id, _)| *id)
        .collect()
}

/// Marks `widget` as an item's own press area: a drag never starts on it, so its click, long press and menu keep
/// working. `id` is the item it shows; `None` for a cell that cannot be selected (a placeholder, a button, a slot).
pub fn mark(widget: &impl IsA<gtk::Widget>, id: Option<i64>) {
    widget.add_css_class(ITEM_CLASS);
    widget.set_widget_name(&id.map_or_else(String::new, |id| format!("{NAME_PREFIX}{id}")));
}

fn id_of(widget: &gtk::Widget) -> Option<i64> {
    widget.widget_name().strip_prefix(NAME_PREFIX).and_then(|id| id.parse().ok())
}

/// What the drag asks of the page. Each call may redraw the view; none runs while the helper holds a borrow.
pub struct Hooks {
    pub is_selected: Rc<dyn Fn(i64) -> bool>,
    /// Flips one id: the same call a Ctrl+click makes.
    pub toggle: Rc<dyn Fn(i64)>,
    /// Empties the selection: a drag without Ctrl replaces it.
    pub clear: Rc<dyn Fn()>,
}

/// One drag in progress.
struct Drag {
    start: (f64, f64),
    keep: bool,
    active: bool,
    /// Each item's state before the drag, taken the first time the item is seen.
    before: HashMap<i64, bool>,
}

/// The overlay that holds the scrolled view and draws the drag box over it. The caller puts `widget()` where the
/// scroller was; the overlay shows and hides with it.
#[derive(Clone)]
pub struct DragSelect {
    overlay: gtk::Overlay,
    band: gtk::Box,
}

impl DragSelect {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let band = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        band.add_css_class("kb-drag-band");
        band.set_can_target(false);
        band.set_halign(gtk::Align::Start);
        band.set_valign(gtk::Align::Start);
        band.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(child));
        overlay.add_overlay(&band);
        overlay.set_clip_overlay(&band, true);
        child.bind_property("visible", &overlay, "visible").sync_create().build();
        Self { overlay, band }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.overlay
    }

    /// Lets a drag on `view` (a `GridView` or `ListView` inside this overlay) select its items.
    pub fn attach(&self, view: &impl IsA<gtk::Widget>, hooks: Hooks) {
        let view: gtk::Widget = view.upcast_ref::<gtk::Widget>().clone();
        let hooks = Rc::new(hooks);
        let state: Rc<RefCell<Option<Drag>>> = Rc::default();
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        gesture.connect_drag_begin({
            let (view, state) = (view.clone(), Rc::clone(&state));
            move |gesture, x, y| {
                if on_item(&view, x, y) {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                let keep = gesture.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
                *state.borrow_mut() = Some(Drag { start: (x, y), keep, active: false, before: HashMap::new() });
            }
        });
        gesture.connect_drag_update({
            let (view, state, hooks, this) = (view.clone(), Rc::clone(&state), Rc::clone(&hooks), self.clone());
            move |gesture, dx, dy| {
                let Some((start, keep, active)) = state.borrow().as_ref().map(|d| (d.start, d.keep, d.active)) else { return };
                if !active {
                    if dx.hypot(dy) < THRESHOLD {
                        return;
                    }
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    if let Some(drag) = state.borrow_mut().as_mut() {
                        drag.active = true;
                    }
                    if !keep {
                        (hooks.clear)();
                    }
                    this.band.set_visible(true);
                }
                let (width, height) = (f64::from(view.width()), f64::from(view.height()));
                let band = Rect::spanning(start.0, start.1, (start.0 + dx).clamp(0.0, width), (start.1 + dy).clamp(0.0, height));
                this.place(&view, band);
                let items = items_on_screen(&view);
                let now: Vec<(i64, bool)> = items.iter().map(|(id, _)| (*id, (hooks.is_selected)(*id))).collect();
                let flip = {
                    let mut state = state.borrow_mut();
                    let Some(drag) = state.as_mut() else { return };
                    for (id, selected) in &now {
                        drag.before.entry(*id).or_insert(*selected);
                    }
                    flips(&now, &drag.before, &hits(band, &items))
                };
                for id in flip {
                    (hooks.toggle)(id);
                }
            }
        });
        gesture.connect_drag_end({
            let (state, band) = (Rc::clone(&state), self.band.clone());
            move |_, _, _| {
                state.borrow_mut().take();
                band.set_visible(false);
            }
        });
        gesture.connect_cancel({
            let (state, band) = (Rc::clone(&state), self.band.clone());
            move |_, _| {
                state.borrow_mut().take();
                band.set_visible(false);
            }
        });
        view.add_controller(gesture);
    }

    /// Moves the drawn box to `band`, given in `view`'s coordinates.
    fn place(&self, view: &gtk::Widget, band: Rect) {
        let origin = view
            .compute_point(&self.overlay, &graphene::Point::new(band.x as f32, band.y as f32))
            .map_or((band.x, band.y), |p| (f64::from(p.x()), f64::from(p.y())));
        self.band.set_margin_start(origin.0.round() as i32);
        self.band.set_margin_top(origin.1.round() as i32);
        self.band.set_size_request(band.width.round() as i32, band.height.round() as i32);
    }
}

/// Whether the point (in `view`'s coordinates) is on an item's own press area.
fn on_item(view: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut current = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = current {
        if widget == *view {
            return false;
        }
        if widget.has_css_class(ITEM_CLASS) {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// The selectable items `view` shows now, with their boxes in its coordinates, top to bottom and left to right.
fn items_on_screen(view: &gtk::Widget) -> Vec<(i64, Rect)> {
    let mut out = Vec::new();
    let mut stack: Vec<gtk::Widget> = Vec::new();
    let mut child = view.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        stack.push(c);
    }
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(ITEM_CLASS) {
            if let Some(id) = id_of(&widget).filter(|_| widget.is_mapped())
                && let Some(bounds) = widget.compute_bounds(view)
            {
                let rect = Rect {
                    x: f64::from(bounds.x()),
                    y: f64::from(bounds.y()),
                    width: f64::from(bounds.width()),
                    height: f64::from(bounds.height()),
                };
                out.push((id, rect));
            }
            continue;
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
    }
    out.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64) -> Rect {
        Rect { x, y, width: 100.0, height: 150.0 }
    }

    /// Two rows of three covers, 100 by 150, with 10 px between them.
    fn grid() -> Vec<(i64, Rect)> {
        (0..6).map(|i| (i, rect(f64::from(i as i32 % 3) * 110.0, f64::from(i as i32 / 3) * 160.0))).collect()
    }

    #[test]
    fn a_box_in_either_direction_is_the_same_box() {
        assert_eq!(Rect::spanning(50.0, 80.0, 10.0, 20.0), Rect { x: 10.0, y: 20.0, width: 40.0, height: 60.0 });
        assert_eq!(Rect::spanning(10.0, 20.0, 50.0, 80.0), Rect::spanning(50.0, 80.0, 10.0, 20.0));
    }

    #[test]
    fn the_box_hits_every_item_it_touches_and_none_in_the_gaps() {
        // from the gap left of the first cover to the middle of the second cover of the second row
        assert_eq!(hits(Rect::spanning(-5.0, 10.0, 150.0, 200.0), &grid()), vec![0, 1, 3, 4]);
        // only the gap between the first two columns
        assert!(hits(Rect::spanning(102.0, 0.0, 108.0, 300.0), &grid()).is_empty());
        // one column, both rows
        assert_eq!(hits(Rect::spanning(250.0, 5.0, 260.0, 300.0), &grid()), vec![2, 5]);
    }

    #[test]
    fn the_drag_adds_what_it_touches_and_puts_back_what_it_leaves() {
        let before: HashMap<i64, bool> = [(1, false), (2, true), (3, false)].into();
        // the box touches 1 and 3: both are added, 2 stays as it was
        assert_eq!(flips(&[(1, false), (2, true), (3, false)], &before, &[1, 3]), vec![1, 3]);
        // the box shrinks back off 3: 3 goes back to unselected, 1 stays
        assert_eq!(flips(&[(1, true), (2, true), (3, true)], &before, &[1]), vec![3]);
        // a selected item the box touches is never deselected
        assert!(flips(&[(1, true), (2, true), (3, false)], &before, &[1, 2]).is_empty());
    }

    #[test]
    fn an_item_first_seen_mid_drag_counts_as_unselected_before() {
        assert_eq!(flips(&[(9, false)], &HashMap::new(), &[9]), vec![9]);
        assert!(flips(&[(9, false)], &HashMap::new(), &[]).is_empty());
    }

    #[test]
    fn mark_names_a_selectable_item_and_leaves_a_plain_one_unnamed() {
        crate::testing::with_display(|| {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            mark(&card, Some(42));
            assert!(card.has_css_class(ITEM_CLASS));
            assert_eq!(id_of(card.upcast_ref()), Some(42));
            mark(&card, None);
            assert_eq!(id_of(card.upcast_ref()), None);
        });
    }
}
