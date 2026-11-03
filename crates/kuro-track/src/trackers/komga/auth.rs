//! Komga credentials, sent as HTTP Basic auth on every request (option a; T-33a-062).
//!
//! Kotlin sends no `Authorization` header and relies on a cookie in the shared cookie jar. Kurobayomi has no such jar,
//! so the username and password the user enters for the tracker are sent instead. The Komga host comes from the
//! tracking URL, so no server address is stored here.

use reqwest::RequestBuilder;
use reqwest::header::USER_AGENT;

use crate::trackers::http::current_user_agent;

pub struct KomgaAuth {
    username: String,
    password: String,
}

impl KomgaAuth {
    pub fn new(username: String, password: String) -> Self {
        Self { username, password }
    }

    /// True when both values are set. Otherwise no credential is sent and Komga answers 401 (E-49).
    pub fn is_set(&self) -> bool {
        !self.username.is_empty() && !self.password.is_empty()
    }

    /// The request with the user's default User-Agent (`KomgaApi.kt:29-31`, T-33a-091) and the Basic credential when it is set.
    pub fn apply(&self, req: RequestBuilder) -> RequestBuilder {
        let req = req.header(USER_AGENT, current_user_agent());
        if self.is_set() {
            req.basic_auth(&self.username, Some(&self.password))
        } else {
            req
        }
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::AUTHORIZATION;

    use super::*;
    use crate::trackers::http::{
        USER_AGENT as APP_USER_AGENT, USER_AGENT_TEST_LOCK, set_user_agent_source, tracker_client,
    };

    fn built(auth: &KomgaAuth) -> reqwest::Request {
        auth.apply(tracker_client().unwrap().get("http://127.0.0.1:1/x"))
            .build()
            .unwrap()
    }

    #[test]
    fn t_33a_062_both_credentials_send_basic_auth() {
        let req = built(&KomgaAuth::new("u".into(), "p".into()));
        assert_eq!(req.headers()[AUTHORIZATION], "Basic dTpw");
    }

    #[test]
    fn t_33a_062_a_missing_credential_sends_no_authorization_header() {
        assert!(!KomgaAuth::new("u".into(), String::new()).is_set());
        let req = built(&KomgaAuth::new(String::new(), "p".into()));
        assert!(req.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn t_33a_061_every_request_carries_the_app_user_agent() {
        let _guard = USER_AGENT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let req = built(&KomgaAuth::new(String::new(), String::new()));
        assert_eq!(req.headers()[USER_AGENT], APP_USER_AGENT);
    }

    #[test]
    fn t_33a_091_a_request_carries_the_default_user_agent_preference() {
        let _guard = USER_AGENT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_user_agent_source(std::sync::Arc::new(|| "Mine/1.0".to_owned()));
        let req = built(&KomgaAuth::new(String::new(), String::new()));
        crate::trackers::http::clear_user_agent_source();
        assert_eq!(req.headers()[USER_AGENT], "Mine/1.0");
    }
}
