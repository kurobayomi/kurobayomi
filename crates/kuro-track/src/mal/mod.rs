//! MyAnimeList tracker. No GTK type and no
//! OS-specific code here: token storage and the config directory are injected.

pub mod api;
pub mod auth;
pub mod consts;
pub mod dates;
pub mod dto;
pub mod error;
pub mod login;
pub mod messages;
pub mod pkce;
pub mod redirect;
pub mod score;
pub mod search;
pub mod status;
pub mod store;
pub mod token;
pub mod tracker;
pub mod tracker_impl;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
pub(crate) mod testkit;
