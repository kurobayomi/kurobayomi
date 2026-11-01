//! The one banner of the app: an `adw::Banner` with a plain-text title and an optional button. The app-state banners of
//! the shell (`shell::banners`) and the info banner of the browse pages (`browse::banner`) both build and fill it here.

/// A hidden banner reading `title`. `title` is plain text, never markup.
pub fn new(title: &str) -> adw::Banner {
    adw::Banner::builder().title(title).use_markup(false).build()
}

/// Sets what `banner` says and its button (`None` hides the button).
pub fn set(banner: &adw::Banner, title: &str, button: Option<&str>) {
    banner.set_title(title);
    banner.set_button_label(button);
}
