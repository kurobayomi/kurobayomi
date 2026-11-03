//! Image request construction and HTTP status acceptance.

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use kuro_net::{CacheControl, NetworkClient, Request};
use tokio_util::sync::CancellationToken;

use crate::error::ImageError;
use crate::options::CachePolicy;

/// `Cache-Control` of an image request. `no-store` when the network cache may be read.
/// Otherwise `no-cache, only-if-cached`, which the server answers with 504 unless a copy is already cached.
pub fn cache_control(network_cache: CachePolicy) -> CacheControl {
    if network_cache.read_enabled() {
        CacheControl {
            no_store: true,
            ..CacheControl::none()
        }
    } else {
        CacheControl {
            no_cache: true,
            only_if_cached: true,
            ..CacheControl::none()
        }
    }
}

/// A GET for an image. The source headers are copied verbatim, and cancelling `cancel` cancels the call.
pub fn image_request(
    url: &str,
    headers: &HeaderMap,
    network_cache: CachePolicy,
    cancel: &CancellationToken,
) -> Result<Request, ImageError> {
    let mut request = kuro_net::get(url)?
        .headers(headers.clone())
        .cache(cache_control(network_cache));
    request.opts.cancel = cancel.clone();
    Ok(request)
}

/// Accepts 2xx and 304. Any other status is an error whose message is the reason phrase.
pub fn check_status(status: StatusCode) -> Result<(), ImageError> {
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    Err(ImageError::Http {
        code: status.as_u16(),
        reason: status.canonical_reason().unwrap_or_default().to_owned(),
    })
}

/// Runs an image request and returns the body. A cancelled request returns `Cancelled`.
pub async fn fetch_bytes(client: &NetworkClient, request: Request) -> Result<Bytes, ImageError> {
    let response = client.execute(request).await?;
    check_status(response.status)?;
    Ok(response.bytes().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{JPEG, test_client};
    use http::HeaderValue;
    use http::header::{CACHE_CONTROL, HeaderName};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn cache_control_follows_the_network_policy() {
        assert_eq!(
            cache_control(CachePolicy::Enabled),
            CacheControl {
                no_store: true,
                ..CacheControl::none()
            }
        );
        assert_eq!(
            cache_control(CachePolicy::Disabled).to_header(),
            Some(HeaderValue::from_static("no-cache, only-if-cached"))
        );
        assert_eq!(
            cache_control(CachePolicy::Enabled).to_header(),
            Some(HeaderValue::from_static("no-store"))
        );
    }

    #[test]
    fn request_copies_source_headers_and_sets_cache_control() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-source"),
            HeaderValue::from_static("yes"),
        );
        let cancel = CancellationToken::new();
        let req = image_request(
            "https://example.invalid/a.jpg",
            &headers,
            CachePolicy::Disabled,
            &cancel,
        )
        .unwrap();
        assert_eq!(req.headers.get("x-source").unwrap(), "yes");
        assert_eq!(
            req.headers.get(CACHE_CONTROL).unwrap(),
            "no-cache, only-if-cached"
        );
    }

    #[test]
    fn invalid_url_is_a_net_error() {
        let cancel = CancellationToken::new();
        let result = image_request(
            "ftp://x/y",
            &HeaderMap::new(),
            CachePolicy::Enabled,
            &cancel,
        );
        assert!(matches!(result, Err(ImageError::Net(_))));
    }

    #[test]
    fn status_acceptance() {
        assert!(check_status(StatusCode::OK).is_ok());
        assert!(check_status(StatusCode::NOT_MODIFIED).is_ok());
        let err = check_status(StatusCode::NOT_FOUND).unwrap_err();
        assert!(matches!(
            err,
            ImageError::Http { code: 404, ref reason } if reason == "Not Found"
        ));
        assert_eq!(err.to_string(), "Not Found");
    }

    #[tokio::test]
    async fn body_of_a_200_is_returned() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ok.jpg"))
            .and(header("x-source", "yes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(1)
            .mount(&server)
            .await;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-source"),
            HeaderValue::from_static("yes"),
        );
        let cancel = CancellationToken::new();
        let req = image_request(
            &format!("{}/ok.jpg", server.uri()),
            &headers,
            CachePolicy::Enabled,
            &cancel,
        )
        .unwrap();
        let body = fetch_bytes(&test_client(tmp.path()), req).await.unwrap();
        assert_eq!(&body[..], JPEG);
    }

    #[tokio::test]
    async fn not_modified_is_accepted_and_other_errors_carry_the_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nm.jpg"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/nf.jpg"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = test_client(tmp.path());
        let cancel = CancellationToken::new();
        let nm = image_request(
            &format!("{}/nm.jpg", server.uri()),
            &HeaderMap::new(),
            CachePolicy::Enabled,
            &cancel,
        )
        .unwrap();
        assert!(fetch_bytes(&client, nm).await.unwrap().is_empty());
        let nf = image_request(
            &format!("{}/nf.jpg", server.uri()),
            &HeaderMap::new(),
            CachePolicy::Enabled,
            &cancel,
        )
        .unwrap();
        match fetch_bytes(&client, nf).await {
            Err(ImageError::Http { code, reason }) => {
                assert_eq!(code, 404);
                assert_eq!(reason, "Not Found");
            }
            other => panic!("expected HTTP 404, got {other:?}"),
        }
    }
}
