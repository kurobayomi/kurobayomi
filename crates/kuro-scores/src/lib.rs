//! Community scores: the public score of a catalogue site for a manga or an anime, to help decide what to read or watch
//! next. No GTK here.
//!
//! One store holds them (`kuro_data::community_score`), and the sources of scores are plug-ins behind one trait
//! ([`provider::ScoreProvider`]): [`jikan`] (the MyAnimeList score, no login) is the first. Adding a site means adding a
//! module that implements the trait; the store, the expiry rules ([`policy`]) and the refresh of a batch ([`refresh`])
//! stay as they are.

pub mod jikan;
pub mod policy;
pub mod provider;
pub mod refresh;
