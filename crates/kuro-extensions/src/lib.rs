//! The repository link side of the plug-ins: the index of a repository (`repo`) and the instantiate step of the plug-in
//! it names (`instantiate`). A source comes from a repository link only.

pub mod repo;

#[cfg(feature = "wasm")]
pub mod instantiate;
#[cfg(not(feature = "wasm"))]
#[path = "instantiate_off.rs"]
pub mod instantiate;
