//! Community scores: the public score of a catalogue site for a manga or an anime, to help decide what to read or watch
//! next. No GTK here.
//!
//! One store holds them (`kuro_data::community_score`), and the sources of scores are plug-ins behind one trait
//! ([`provider::ScoreProvider`]): [`mal`] (the MyAnimeList score, from MyAnimeList's own API) is the one in use. Jikan,
//! the first, was removed: its public API was shut down on 2026-10-01. Adding a site means adding a
//! module that implements the trait; the store, the expiry rules ([`policy`]) and the refresh of a batch ([`refresh`])
//! stay as they are.

pub mod mal;
pub mod policy;
pub mod provider;
pub mod refresh;
