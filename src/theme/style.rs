//! The `kb-*` component classes and their rules (`p2-rust-design.md` section 5.5), the base
//! stylesheet, and the pure alpha and padding helpers of T-49b-028 and T-49b-029.


use super::tokens::Tokens;

/// The base stylesheet: GTK named colours and static rules, loaded before the palette.
pub const BASE_CSS: &str = include_str!("style.css");

/// CSS class names for the Kurobayomi component classes.
pub mod class {
    pub const SURFACE: &str = "kb-surface";
    pub const SELECTED: &str = "kb-selected";
    pub const ACTIVE: &str = "kb-active";
    pub const PILL: &str = "kb-pill";
    pub const PLAYER_RIPPLE: &str = "kb-player-ripple";
    pub const TAB_PILL: &str = "kb-tab-pill";
    pub const HIT_TARGET: &str = "kb-hit-target";
    pub const NOINDICATION: &str = "kb-noindication";
    pub const INFO_BAR: &str = "kb-info-bar";
    pub const DRAGGED: &str = "kb-dragged";
    pub const FAST_THUMB: &str = "kb-fast-thumb";
    pub const FAST_SHOWN: &str = "kb-fast-shown";
    pub const BUTTON_FILLED: &str = "kb-button-filled";
    pub const BUTTON_TEXT: &str = "kb-button-text";
    pub const SHEET: &str = "kb-sheet";
    /// A glyph in the primary colour (the info screen icon).
    pub const PRIMARY_ICON: &str = "kb-primary-icon";
    /// The app bar's count pill (shell/appbar.rs); its geometry is in style.css, its alpha in `system_rules`.
    pub const COUNT_PILL: &str = "kuro-count-pill";
}

/// Alpha of the onSurface state layer under a hovered clickable surface; a press uses
/// `RIPPLE_ALPHA` (the Compose ripple defaults, T-49b-021).
pub const HOVER_ALPHA: f32 = 0.08;

/// Alpha of the selected-row tint: 0.16 in dark, 0.22 in light (`selectedBackground`, M1).
pub fn selected_alpha(dark: bool) -> f32 {
    if dark { 0.16 } else { 0.22 }
}

/// The dark-aware rules for the GTK-theme choice: the selection tint keeps the GTK theme's own
/// selected colour, the active tint follows the resolved dark flag (T-49b-010, T-49b-029), and the
/// pill takes a GTK-theme stand-in for `surface_container_high` (T-49a-034). The app bar's count pill
/// takes the same pill alpha over the window foreground.
pub fn system_rules(dark: bool) -> String {
    let active = Tokens::active_for(dark).to_css();
    // the tab pill follows the generic pill, so its alpha background wins at equal specificity
    format!(
        ".{} {{ background-color: alpha(@theme_selected_bg_color, {}); }}\n.{} {{ color: {active}; }}\n.{} {{ background-color: mix(@theme_bg_color, @theme_fg_color, 0.1); }}\n.{} {{ background-color: alpha(@theme_fg_color, {}); }}\n.{} {{ color: @accent_color; }}\n.{}:hover {{ background-color: alpha(@theme_fg_color, {HOVER_ALPHA}); }}\n.{}:active {{ background-color: alpha(@theme_fg_color, {}); }}\n.{count_pill} {{ background-color: alpha(@window_fg_color, {count_alpha}); }}\n",
        class::SELECTED,
        selected_alpha(dark),
        class::ACTIVE,
        class::PILL,
        class::TAB_PILL,
        crate::widgets::tabs::pill_alpha(dark),
        class::PRIMARY_ICON,
        class::SURFACE,
        class::SURFACE,
        crate::theme::tokens::RIPPLE_ALPHA,
        count_pill = class::COUNT_PILL,
        count_alpha = crate::widgets::tabs::pill_alpha(dark),
    )
}

/// The fast-scroll thumb (T-49a-031): hidden by default, shown with a class, and faded by a CSS
/// transition of `SCROLLBAR_FADE_MS`. Its colour is the primary accent.
pub fn fast_thumb_rules() -> String {
    format!(
        ".{thumb} {{ opacity: 0; border-radius: 6px; background-color: @accent_color; transition: opacity {fade}ms linear; }}\n.{thumb}.{shown} {{ opacity: 1; transition: none; }}\n",
        thumb = class::FAST_THUMB,
        shown = class::FAST_SHOWN,
        fade = crate::theme::tokens::SCROLLBAR_FADE_MS,
    )
}

/// The rules that depend on the system dark flag or the tokens of the widget layer, added to the
/// tint provider for every choice.
pub fn widget_rules(system_dark: bool) -> String {
    format!("{}{}{}", ripple_rules(system_dark), fast_thumb_rules(), widget_static_rules())
}

/// The widget classes of `widgets/`. They use GTK named colours only, so they follow the GTK theme
/// and a picked scheme alike (the scheme overrides the named colours).
pub fn widget_static_rules() -> String {
    format!(
        "\
.{filled} {{ background-color: @accent_bg_color; color: @accent_fg_color; min-width: 58px; min-height: 40px; border-radius: 999px; padding: 0 24px; }}
.{filled}:hover {{ box-shadow: 0 1px 2px alpha(black, 0.3); }}
.{filled}:disabled {{ background-color: alpha(@accent_bg_color, 0.12); color: alpha(@window_fg_color, 0.38); }}
.{text} {{ background: none; color: @accent_color; padding: 0 12px; }}
.{text}:disabled {{ color: alpha(@window_fg_color, 0.38); }}
.{sheet} {{ background-color: @popover_bg_color; border-radius: 28px 28px 0 0; padding: 24px; }}
.{pill} {{ padding: 1px 6px; border-radius: 999px; }}
.{hit} {{ min-width: 48px; min-height: 48px; }}
.{noind}:hover, .{noind}:active {{ background-image: none; background-color: transparent; }}
.{info_bar} {{ border-top: 1px solid alpha(@window_fg_color, 0.12); padding: 8px 0 16px 0; }}
",
        filled = class::BUTTON_FILLED,
        text = class::BUTTON_TEXT,
        sheet = class::SHEET,
        pill = class::PILL,
        hit = class::HIT_TARGET,
        noind = class::NOINDICATION,
        info_bar = class::INFO_BAR,
    )
}

/// The player state layer (T-49a-010): `RIPPLE_ALPHA` over hover, focus-visible, active and drag.
/// White in the system dark mode and black in the system light mode, never the app scheme.
pub fn ripple_rules(system_dark: bool) -> String {
    let rgb = if system_dark { "255, 255, 255" } else { "0, 0, 0" };
    format!(
        ".{layer}:hover, .{layer}:focus-visible, .{layer}:active, .{layer}.{dragged} {{ background-color: rgba({rgb}, {alpha}); }}\n",
        layer = class::PLAYER_RIPPLE,
        dragged = class::DRAGGED,
        alpha = crate::theme::tokens::RIPPLE_ALPHA,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::Rgba;

    #[test]
    fn system_rules_follow_the_dark_flag() {
        let dark = system_rules(true);
        let light = system_rules(false);
        assert!(dark.contains("alpha(@theme_selected_bg_color, 0.16)"));
        assert!(light.contains("alpha(@theme_selected_bg_color, 0.22)"));
        assert!(dark.contains(&format!(".kb-active {{ color: {}; }}", Rgba::argb(0xFFFF_EB3B).to_css())));
        assert!(light.contains(&format!(".kb-active {{ color: {}; }}", Rgba::argb(0xFFFF_C107).to_css())));
    }

    // T-49a-034: the pill geometry is always on; the colour is a GTK mix.
    #[test]
    fn pill_geometry_is_always_on_and_colour_is_a_gtk_mix() {
        let rules = widget_rules(false);
        assert!(rules.contains(".kb-pill { padding: 1px 6px; border-radius: 999px; }"));
        assert!(system_rules(true).contains(".kb-pill { background-color: mix(@theme_bg_color"));
    }

    // T-49b-021: an indicated surface has a hover and a press background; .kb-noindication has none.
    #[test]
    fn clickable_surface_has_hover_and_press_backgrounds() {
        assert!(system_rules(false).contains(".kb-surface:hover { background-color: alpha(@theme_fg_color, 0.08); }"));
        assert!(system_rules(false).contains(".kb-surface:active { background-color: alpha(@theme_fg_color, 0.1); }"));
        assert!(widget_static_rules().contains(".kb-noindication:hover, .kb-noindication:active { background-image: none; background-color: transparent; }"));
    }

    // T-49b-022: the tab pill rule comes after the generic pill rule, so its alpha background wins.
    #[test]
    fn tab_pill_rule_follows_the_pill_rule() {
        for dark in [false, true] {
            let system = system_rules(dark);
            assert!(system.find(".kb-tab-pill {").unwrap() > system.find(".kb-pill {").unwrap(), "dark={dark}");
        }
    }

    // T-49b-026: the info icon is primary, not the amber filter tint.
    #[test]
    fn primary_icon_class_uses_the_primary_role() {
        assert!(system_rules(true).contains(".kb-primary-icon { color: @accent_color; }"));
    }

    #[test]
    fn base_stylesheet_leaves_dark_aware_rules_to_the_generated_css() {
        assert!(!BASE_CSS.contains(".kb-active") && !BASE_CSS.contains(".kb-selected"));
    }

    #[test]
    fn base_stylesheet_braces_balance() {
        assert_eq!(BASE_CSS.matches('{').count(), BASE_CSS.matches('}').count());
    }
}

#[cfg(test)]
mod ripple_tests {
    use super::{fast_thumb_rules, ripple_rules};

    #[test]
    fn fast_thumb_fades_over_the_scrollbar_duration() {
        let css = fast_thumb_rules();
        assert!(css.contains("transition: opacity 250ms"));
        assert!(css.contains(".kb-fast-thumb.kb-fast-shown"));
    }

    #[test]
    fn ripple_is_white_in_system_dark_and_black_in_system_light() {
        // T-49a-010 / MANUAL-49-07
        assert!(ripple_rules(true).contains("rgba(255, 255, 255, 0.1)"));
        assert!(ripple_rules(false).contains("rgba(0, 0, 0, 0.1)"));
    }

    #[test]
    fn ripple_covers_hover_focus_active_and_drag() {
        let css = ripple_rules(true);
        for state in [":hover", ":focus-visible", ":active", ".kb-dragged"] {
            assert!(css.contains(state), "{state}");
        }
    }

    // T-49b-013: `.kb-player-ripple`, alpha .1 for every state.
    #[test]
    fn player_ripple_class_and_alpha() {
        assert_eq!(super::class::PLAYER_RIPPLE, "kb-player-ripple");
        assert_eq!(crate::theme::tokens::RIPPLE_ALPHA, 0.1);
        let css = ripple_rules(false);
        assert!(css.contains(".kb-player-ripple:hover"));
        assert!(css.contains(".kb-player-ripple.kb-dragged"));
    }
}


#[cfg(test)]
mod parse_tests {
    /// The widget and system rules parse under `gtk::CssProvider`. Needs a display, and skips
    /// itself without one.
    #[test]
    fn widget_rules_parse_under_css_provider() {
        crate::testing::with_display(|| {
            for dark in [false, true] {
                let css = format!("{}{}", super::system_rules(dark), super::widget_rules(dark));
                if let Err(message) = crate::theme::apply::build_provider(&css) {
                    panic!("CSS did not parse: {message}");
                }
            }
        });
    }
}

#[cfg(test)]
mod base_css_tests {
    use super::BASE_CSS;

    fn has(rule: &str) -> bool {
        BASE_CSS.contains(rule)
    }

    /// T-49ba-045: the base sheet names only GTK named colours, or a palette colour that the tint provider defines
    /// (`kuro_<role>`), so no rule refers to a colour that does not exist.
    #[test]
    fn base_sheet_references_only_gtk_named_colours_and_palette_roles() {
        const GTK: [&str; 9] = [
            "theme_bg_color",
            // the standard border colour (GTK's default theme and Breeze define it): the dialog outline
            "borders",
            "theme_fg_color",
            "theme_base_color",
            "theme_selected_bg_color",
            "theme_selected_fg_color",
            "error_color",
            "success_color",
            "warning_color",
        ];
        let mut rest = BASE_CSS;
        while let Some(at) = rest.find('@') {
            let name: String = rest[at + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            // an at-rule, not a colour: the refresh arrow's animation
            if name == "keyframes" {
                rest = &rest[at + 1..];
                continue;
            }
            let palette = name
                .strip_prefix("kuro_")
                .is_some_and(|role| crate::theme::tokens::Role::ALL.iter().any(|r| r.css_name() == role));
            assert!(GTK.contains(&name.as_str()) || palette, "undefined colour @{name} in the base sheet");
            rest = &rest[at + 1..];
        }
    }

    /// The declarations of the base-sheet rule for `selector`, on one line, or `None` when the sheet has no such rule.
    fn declarations(selector: &str) -> Option<String> {
        let start = BASE_CSS.find(&format!("{selector} {{"))? + selector.len() + 2;
        let end = BASE_CSS[start..].find('}')? + start;
        Some(BASE_CSS[start..end].split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// The rules of the screens that used to add their own provider now live in the base sheet, with the same
    /// declarations. Nothing outside `theme` adds a provider (`apply::no_screen_adds_a_css_provider`).
    #[test]
    fn screen_rules_live_in_the_base_sheet() {
        let rules = [
            (".pref-flash", "background-color: alpha(@theme_selected_bg_color, 0.18);"),
            (".pref-error", "color: @error_color;"),
            (".tracker-check", "color: @success_color;"),
            (".reader-bg-black", "background-color: #000000;"),
            (".reader-bg-gray", "background-color: #202125;"),
            (".reader-scrim", "background-color: rgba(0, 0, 0, 0.5);"),
            (".reader-text-light", "color: #ffffff;"),
            (".kuro-backdrop-fade", "linear-gradient(to bottom, alpha(@theme_bg_color, 0.3) 0%, alpha(@theme_bg_color, 0.45) 60%, @theme_bg_color 100%);"),
            (".kb-banner", "background-color: alpha(@theme_selected_bg_color, 0.18); color: @theme_fg_color; padding: 4px 16px;"),
            (".kb-banner.kb-banner-error", "background-color: alpha(@error_color, 0.18); padding: 16px;"),
            (".kuro-cal-day:focus-visible", "outline: 2px solid @theme_selected_bg_color;"),
            (".kuro-cal-dot", "background-color: @theme_selected_bg_color;"),
        ];
        for (selector, declaration) in rules {
            let body = declarations(selector).unwrap_or_else(|| panic!("{selector} is not in style.css"));
            assert!(body.contains(declaration), "{selector}: {body}");
        }
    }

    /// The count pill keeps its geometry in the base sheet; its alpha depends on the dark flag, so it is
    /// generated with the other dark-aware rules.
    #[test]
    fn count_pill_alpha_follows_the_dark_flag() {
        assert!(declarations(".kuro-count-pill").is_some_and(|b| b.contains("border-radius: 999px;")));
        assert!(super::system_rules(true).contains(".kuro-count-pill { background-color: alpha(@window_fg_color, 0.12); }"));
        assert!(super::system_rules(false).contains(".kuro-count-pill { background-color: alpha(@window_fg_color, 0.08); }"));
    }

    /// T-49ba-027: a dragged or selected download card gets alpha 0.08 of the selection colour.
    #[test]
    fn draggable_cards_tint_with_the_selection_colour() {
        assert!(has(".kb-draggable.kb-dragged,"));
        assert!(has("alpha(@theme_selected_bg_color, 0.08)"));
    }

    /// T-49ba-049: the modal scrim is black at 0.32.
    #[test]
    fn scrim_is_black_at_032() {
        assert!(has(".kb-scrim {\n  background-color: rgba(0, 0, 0, 0.32);"));
    }

    /// T-49ba-020: the splash colours are the Android splash colours.
    #[test]
    fn splash_colours_match_the_android_resources() {
        assert!(has(".kb-splash {\n  background-color: #54759E;"));
        assert!(has(".kb-splash.kb-dark {\n  background-color: #202125;"));
    }

    /// T-49ba-032: dividers are one pixel of the foreground at 0.12.
    #[test]
    fn dividers_are_one_pixel_at_012() {
        assert!(has(".kb-divider, separator.kb-divider {\n  min-height: 1px;\n  background-color: alpha(@theme_fg_color, 0.12);"));
    }

    /// T-49ba-022: the tab-enter rotation classes that `anim::rotate_icon` sets.
    #[test]
    fn rotation_classes_are_defined() {
        for class in ["kb-rot-0 {", "kb-rot-180 {", "kb-rot-360 {", "kb-icon-rot.kb-rot-0-slow {"] {
            assert!(has(class), "{class}");
        }
        assert!(has("transition: -gtk-icon-transform 300ms cubic-bezier(0.4, 0, 0.2, 1);"));
    }
}
