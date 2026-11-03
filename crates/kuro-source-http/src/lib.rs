//! HTTP layer of the source API (area 15). `kuro-net` does the transport, the
//! interceptors and the cookies. This crate adds the source rules on top: request builders, the success check and
//! error mapping, URL helpers, the jsoup-compatible HTML helpers, and the `HttpCore` state.

pub mod error;
pub mod html;
pub mod http_source;
pub mod local_server;
pub mod request;
pub mod response;
pub mod urls;

pub use error::map_net_error;
pub use http_source::HttpCore;
pub use local_server::{LocalHandler, LocalResponse, LocalStreamServer};
pub use response::HttpResponse;
