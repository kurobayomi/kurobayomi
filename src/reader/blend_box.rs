//! A container that blends a colour over its child with a `gsk` blend mode. GTK's CSS has no `mix-blend-mode`, and a
//! plain overlay widget cannot see what is under it, so the blend is drawn here: the child is the bottom image of the
//! blend and the colour is the top image. Used for the colour filter layer (T-043a-066). The child is also drawn through
//! a colour-matrix node for grayscale and invert (T-43b-049), under the blend, so the page area alone is filtered.

use gtk::glib;
use gtk::prelude::*;

mod imp {
    use std::cell::Cell;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib, graphene, gsk};

    #[derive(Default)]
    pub struct BlendBox {
        pub colour: Cell<Option<(f32, f32, f32, f32)>>,
        pub mode: Cell<i32>,
        /// The grayscale and invert matrix of the page area, when either is on.
        pub page_filter: Cell<Option<crate::reader::overlay::ColourMatrix>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BlendBox {
        const NAME: &'static str = "KuroReaderBlendBox";
        type Type = super::BlendBox;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for BlendBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for BlendBox {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let Some(child) = obj.first_child() else { return };
            let draw_child = || super::with_page_filter(snapshot, self.page_filter.get(), || obj.snapshot_child(&child, snapshot));
            let Some((r, g, b, a)) = self.colour.get() else {
                draw_child();
                return;
            };
            snapshot.push_blend(super::blend_mode(self.mode.get()));
            draw_child();
            snapshot.pop();
            snapshot.append_color(&gdk::RGBA::new(r, g, b, a), &graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32));
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct BlendBox(ObjectSubclass<imp::BlendBox>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The `gsk` blend mode of a colour filter mode index (`0..=5`: default, multiply, screen, overlay, lighten, darken).
/// An index outside the list is the default mode.
pub fn blend_mode(mode: i32) -> gtk::gsk::BlendMode {
    use gtk::gsk::BlendMode;
    match mode {
        1 => BlendMode::Multiply,
        2 => BlendMode::Screen,
        3 => BlendMode::Overlay,
        4 => BlendMode::Lighten,
        5 => BlendMode::Darken,
        _ => BlendMode::Default,
    }
}

/// Runs `draw` inside a `GskColorMatrixNode` for `filter`, or plainly when there is none.
pub fn with_page_filter(snapshot: &gtk::Snapshot, filter: Option<crate::reader::overlay::ColourMatrix>, draw: impl FnOnce()) {
    let Some((matrix, offset)) = filter else {
        draw();
        return;
    };
    snapshot.push_color_matrix(&gtk::graphene::Matrix::from_float(matrix), &gtk::graphene::Vec4::from_float(offset));
    draw();
    snapshot.pop();
}

impl BlendBox {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let this: Self = glib::Object::new();
        // a custom container does not forward its child's expand flags
        this.set_hexpand(true);
        this.set_vexpand(true);
        child.set_parent(&this);
        this
    }

    /// Sets the grayscale and invert matrix of the page area; `None` draws the child as is.
    pub fn set_page_filter(&self, filter: Option<crate::reader::overlay::ColourMatrix>) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().page_filter.set(filter);
        self.queue_draw();
    }

    /// Sets the colour (red, green, blue, alpha in `0..=1`) and the mode index; `None` shows the child as is.
    pub fn set_filter(&self, colour: Option<(f64, f64, f64, f64)>, mode: i32) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let state = self.imp();
        state.colour.set(colour.map(|(r, g, b, a)| (r as f32, g as f32, b as f32, a as f32)));
        state.mode.set(mode);
        self.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node a snapshot of one red square makes with `filter`.
    fn node_with(filter: Option<crate::reader::overlay::ColourMatrix>) -> gtk::gsk::RenderNode {
        let snapshot = gtk::Snapshot::new();
        with_page_filter(&snapshot, filter, || {
            snapshot.append_color(&gtk::gdk::RGBA::new(1.0, 0.0, 0.0, 1.0), &gtk::graphene::Rect::new(0.0, 0.0, 4.0, 4.0));
        });
        snapshot.to_node().expect("a node")
    }

    #[test]
    fn t066_with_no_filter_the_page_is_drawn_without_a_matrix_node() {
        crate::testing::with_display(|| {
            assert!(node_with(None).downcast_ref::<gtk::gsk::ColorNode>().is_some());
        });
    }
}
