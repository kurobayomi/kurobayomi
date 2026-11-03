//! Kitsu login and logout dialog texts and rules (T-30a-008). The GUI shows these
//! texts as they are.

pub const LOGIN_TITLE: &str = "Log in to Kitsu";
pub const USERNAME_LABEL: &str = "Email address";
pub const PASSWORD_LABEL: &str = "Password";
pub const LOGIN_BUTTON: &str = "Login";
pub const LOGIN_PROCESSING: &str = "Loading…";
pub const LOGGED_IN_NOTICE: &str = "Logged in";
pub const LOGOUT_TITLE: &str = "Log out from Kitsu?";
pub const CANCEL_BUTTON: &str = "Cancel";
pub const LOGOUT_BUTTON: &str = "Log out";
pub const LOGGED_OUT_NOTICE: &str = "You are now logged out";

/// The login button is enabled only when no login runs and both fields have text other than spaces.
pub fn login_can_submit(username: &str, password: &str, processing: bool) -> bool {
    !processing && !username.trim().is_empty() && !password.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_30a_008_login_needs_both_fields_and_no_running_login() {
        assert!(login_can_submit("ann@example.test", "pw", false));
        assert!(!login_can_submit("   ", "pw", false));
        assert!(!login_can_submit("ann", "", false));
        assert!(!login_can_submit("ann", "pw", true));
    }

    #[test]
    fn t_30a_008_dialog_texts_are_the_source_strings() {
        assert_eq!(LOGIN_TITLE, "Log in to Kitsu");
        assert_eq!(LOGOUT_TITLE, "Log out from Kitsu?");
        assert_eq!(LOGGED_OUT_NOTICE, "You are now logged out");
    }
}
