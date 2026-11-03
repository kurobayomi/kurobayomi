//! The application interceptor chain. An interceptor sees the request, may
//! change it, calls `chain.proceed` once, and may change the response. The last step is the transport.

pub mod cloudflare;
pub mod rate_limit;
pub mod safe_headers;
pub mod uncaught;
pub mod user_agent;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::NetError;
use crate::request::Request;
use crate::response::Response;
use crate::transport::Transport;

pub use cloudflare::CloudflareInterceptor;
pub use rate_limit::RateLimit;
pub use uncaught::UncaughtExceptionInterceptor;
pub use user_agent::UserAgentInterceptor;

#[async_trait]
pub trait Interceptor: Send + Sync {
    async fn intercept(&self, chain: Chain<'_>, req: Request) -> Result<Response, NetError>;
}

/// The rest of the chain: the interceptors not yet run, then the transport. Copying it is cheap.
#[derive(Clone, Copy)]
pub struct Chain<'a> {
    rest: &'a [Arc<dyn Interceptor>],
    transport: &'a Transport,
}

impl<'a> Chain<'a> {
    pub(crate) fn new(rest: &'a [Arc<dyn Interceptor>], transport: &'a Transport) -> Self {
        Self { rest, transport }
    }

    /// Runs the next interceptor, or the transport when none is left.
    pub async fn proceed(self, req: Request) -> Result<Response, NetError> {
        match self.rest.split_first() {
            Some((next, rest)) => {
                next.intercept(
                    Chain {
                        rest,
                        transport: self.transport,
                    },
                    req,
                )
                .await
            }
            None => self.transport.send(req).await,
        }
    }
}
