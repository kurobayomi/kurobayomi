//! HTTP stack for Kurobayomi (area 12). Built on reqwest. Interceptors are kept as an ordered chain, as OkHttp
//! has them, so extensions can add one. No GTK and no OS code in this crate: window work goes through
//! `kuro_platform::webview::WebViewHost` and `kuro_platform::toast::Toaster`.

pub mod cache;
pub mod client;
pub mod cookies;
pub mod doh;
pub mod error;
pub mod interceptors;
#[cfg(feature = "js-sources")]
pub mod js;
mod logging;
pub mod page_gate;
pub mod progress;
pub mod public_only;
pub mod request;
pub mod response;
pub mod settings;
pub mod transport;

pub use client::{ClientBuilder, DEFAULT_RATE_PERIOD, NetworkClient, UserAgentSource};
pub use cookies::CookieJar;
pub use error::{NetError, UserMessage};
pub use interceptors::{Chain, Interceptor, RateLimit};
pub use progress::ProgressListener;
pub use request::{Body, CacheControl, CallOptions, Method, Request, get, post};
pub use response::Response;
pub use settings::NetworkSettings;
