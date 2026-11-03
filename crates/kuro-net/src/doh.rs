//! DNS over HTTPS (area 12, T-12a-024 and T-12a-025).
//!
//! The provider table and its bootstrap addresses are the Kotlin `DohProviders.kt` values. The resolver is
//! plugged into the reqwest client as its DNS hook: A and AAAA queries are sent concurrently in RFC 8484 GET form,
//! and the provider host itself is pinned to its bootstrap addresses so it never resolves through DoH.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::join_all;
use hickory_resolver::proto::op::{Message, Query, ResponseCode};
use hickory_resolver::proto::rr::rdata::{A, AAAA};
use hickory_resolver::proto::rr::{Name as DnsName, RData, RecordType};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::Url;

use crate::error::NetError;

/// The largest DoH answer the resolver reads (`DnsOverHttps.readResponse`).
pub const MAX_RESPONSE_BYTES: usize = 65_536;

/// Every `doh_provider` id from 1 to 13. The variants keep the Kotlin preference ids as discriminants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DohProvider {
    Cloudflare = 1,
    Google = 2,
    AdGuard = 3,
    Quad9 = 4,
    AliDns = 5,
    DnsPod = 6,
    /// The Kotlin `doh360`, named after its host `doh.360.cn`.
    Qihoo360 = 7,
    /// The Kotlin `dohQuad101`, which points at TWNIC's resolver.
    Quad101 = 8,
    Mullvad = 9,
    ControlD = 10,
    Njalla = 11,
    Shecan = 12,
    LibreDns = 13,
}

impl DohProvider {
    pub const ALL: [DohProvider; 13] = [
        DohProvider::Cloudflare,
        DohProvider::Google,
        DohProvider::AdGuard,
        DohProvider::Quad9,
        DohProvider::AliDns,
        DohProvider::DnsPod,
        DohProvider::Qihoo360,
        DohProvider::Quad101,
        DohProvider::Mullvad,
        DohProvider::ControlD,
        DohProvider::Njalla,
        DohProvider::Shecan,
        DohProvider::LibreDns,
    ];

    /// The provider for a `doh_provider` preference. `-1` (disabled), `0`, `14` and other ids give `None`, which
    /// means the system resolver.
    pub fn from_pref(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn id(self) -> i32 {
        self as i32
    }

    /// The DoH endpoint. Its host is the one the bootstrap addresses pin.
    pub fn url(self) -> &'static str {
        match self {
            DohProvider::Cloudflare => "https://cloudflare-dns.com/dns-query",
            DohProvider::Google => "https://dns.google/dns-query",
            DohProvider::AdGuard => "https://dns-unfiltered.adguard.com/dns-query",
            DohProvider::Quad9 => "https://dns.quad9.net/dns-query",
            DohProvider::AliDns => "https://dns.alidns.com/dns-query",
            DohProvider::DnsPod => "https://doh.pub/dns-query",
            DohProvider::Qihoo360 => "https://doh.360.cn/dns-query",
            DohProvider::Quad101 => "https://dns.twnic.tw/dns-query",
            DohProvider::Mullvad => "https://dns.mullvad.net/dns-query",
            DohProvider::ControlD => "https://freedns.controld.com/p0",
            DohProvider::Njalla => "https://dns.njal.la/dns-query",
            DohProvider::Shecan => "https://free.shecan.ir/dns-query",
            DohProvider::LibreDns => "https://doh.libredns.gr/dns-query",
        }
    }

    /// The bootstrap addresses, in the Kotlin order.
    pub fn bootstrap(self) -> &'static [&'static str] {
        match self {
            DohProvider::Cloudflare => &[
                "162.159.36.1",
                "162.159.46.1",
                "1.1.1.1",
                "1.0.0.1",
                "162.159.132.53",
                "2606:4700:4700::1111",
                "2606:4700:4700::1001",
                "2606:4700:4700::0064",
                "2606:4700:4700::6400",
            ],
            DohProvider::Google => &[
                "8.8.4.4",
                "8.8.8.8",
                "2001:4860:4860::8888",
                "2001:4860:4860::8844",
            ],
            DohProvider::AdGuard => &[
                "94.140.14.140",
                "94.140.14.141",
                "2a10:50c0::1:ff",
                "2a10:50c0::2:ff",
            ],
            DohProvider::Quad9 => &["9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9"],
            DohProvider::AliDns => &[
                "223.5.5.5",
                "223.6.6.6",
                "2400:3200::1",
                "2400:3200:baba::1",
            ],
            DohProvider::DnsPod => &["1.12.12.12", "120.53.53.53"],
            DohProvider::Qihoo360 => &[
                "101.226.4.6",
                "218.30.118.6",
                "123.125.81.6",
                "140.207.198.6",
                "180.163.249.75",
                "101.199.113.208",
                "36.99.170.86",
            ],
            DohProvider::Quad101 => &["101.101.101.101", "2001:de4::101", "2001:de4::102"],
            DohProvider::Mullvad => &["194.242.2.2", "2a07:e340::2"],
            DohProvider::ControlD => &["76.76.2.0", "76.76.10.0", "2606:1a40::", "2606:1a40:1::"],
            DohProvider::Njalla => &["95.215.19.53", "2001:67c:2354:2::53"],
            DohProvider::Shecan => &["178.22.122.100", "185.51.200.2"],
            DohProvider::LibreDns => &["116.202.176.26", "2a01:4f8:1c0c:8274::1"],
        }
    }

    /// The bootstrap addresses as sockets on the HTTPS port, the pins of the DoH client.
    pub fn bootstrap_addrs(self) -> Vec<SocketAddr> {
        self.bootstrap()
            .iter()
            .map(|ip| SocketAddr::new(ip.parse().expect("bootstrap table holds IP literals"), 443))
            .collect()
    }
}

/// The name the DoH endpoint is reached under.
fn endpoint_host(url: &Url) -> Result<&str, NetError> {
    url.host_str()
        .ok_or_else(|| NetError::InvalidUrl(format!("DoH endpoint has no host: {url}")))
}

/// `true` when OkHttp would resolve `host` over DoH with its default rule: a name with no public suffix is private
/// and is refused while DoH is on (`DnsOverHttps.lookup`). IP literals are never looked up.
fn is_public_name(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok() || psl::domain_str(host).is_some()
}

/// The RFC 8484 GET query for one record type: wire form with id 0 and recursion desired.
fn encode_query(host: &str, record_type: RecordType) -> Result<Vec<u8>, String> {
    let name = DnsName::from_str(host).map_err(|e| e.to_string())?;
    let mut msg = Message::query();
    msg.metadata.id = 0;
    msg.metadata.recursion_desired = true;
    msg.add_query(Query::query(name, record_type));
    msg.to_vec().map_err(|e| e.to_string())
}

/// The `dns` query parameter: the wire query, base64url without padding.
fn dns_parameter(wire: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(wire)
}

/// The addresses of the answers whose type matches the query.
fn decode_answers(wire: &[u8], record_type: RecordType) -> Result<Vec<IpAddr>, String> {
    let msg = Message::from_vec(wire).map_err(|e| e.to_string())?;
    if msg.metadata.response_code != ResponseCode::NoError {
        return Err(format!("response code {}", msg.metadata.response_code));
    }
    Ok(msg
        .answers
        .iter()
        .filter_map(|record| match (&record.data, record_type) {
            (RData::A(A(ip)), RecordType::A) => Some(IpAddr::V4(*ip)),
            (RData::AAAA(AAAA(ip)), RecordType::AAAA) => Some(IpAddr::V6(*ip)),
            _ => None,
        })
        .collect())
}

fn size_error(bytes: usize) -> String {
    format!("response size exceeds limit ({MAX_RESPONSE_BYTES} bytes): {bytes} bytes")
}

/// A DoH resolver: the endpoint, the client that talks to it, and whether AAAA is asked for.
#[derive(Clone)]
pub struct DohResolver {
    endpoint: Url,
    client: reqwest::Client,
    include_ipv6: bool,
}

impl DohResolver {
    /// The resolver for a provider. The client pins the provider host to its bootstrap addresses and sends the
    /// configured `User-Agent`; it has no interceptors, cookies or cache.
    pub fn new(provider: DohProvider, user_agent: &str) -> Result<Self, NetError> {
        let endpoint =
            Url::parse(provider.url()).map_err(|e| NetError::InvalidUrl(e.to_string()))?;
        let host = endpoint_host(&endpoint)?.to_owned();
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .resolve_to_addrs(&host, &provider.bootstrap_addrs())
            .user_agent(user_agent)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| NetError::Other(format!("cannot build DoH client: {e}")))?;
        Ok(Self::from_parts(endpoint, client, true))
    }

    /// A resolver over an endpoint and client the caller built. `new` uses this with the pinned client.
    pub fn from_parts(endpoint: Url, client: reqwest::Client, include_ipv6: bool) -> Self {
        Self {
            endpoint,
            client,
            include_ipv6,
        }
    }

    /// Looks up `host` over DoH: A and AAAA concurrently, the union of the answers in arrival order without
    /// duplicates. When nothing comes back, the first failure is the error.
    pub async fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        if !is_public_name(host) {
            return Err(io::Error::other("private hosts not resolved"));
        }
        let mut kinds = vec![RecordType::A];
        if self.include_ipv6 {
            kinds.push(RecordType::AAAA);
        }
        let results = join_all(kinds.into_iter().map(|kind| self.query(host, kind))).await;
        let mut addrs: Vec<IpAddr> = Vec::new();
        let mut first_failure = None;
        for result in results {
            match result {
                Ok(found) => {
                    for ip in found {
                        if !addrs.contains(&ip) {
                            addrs.push(ip);
                        }
                    }
                }
                Err(message) => {
                    first_failure.get_or_insert(message);
                }
            }
        }
        if addrs.is_empty() {
            let message = first_failure.unwrap_or_else(|| format!("no address for {host}"));
            return Err(io::Error::other(message));
        }
        Ok(addrs)
    }

    async fn query(&self, host: &str, kind: RecordType) -> Result<Vec<IpAddr>, String> {
        let wire = encode_query(host, kind)?;
        let mut response = self
            .client
            .get(self.endpoint.clone())
            .header(http::header::ACCEPT, "application/dns-message")
            .query(&[("dns", dns_parameter(&wire))])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "response: {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or_default()
            ));
        }
        if let Some(declared) = response.content_length()
            && declared > MAX_RESPONSE_BYTES as u64
        {
            return Err(size_error(declared as usize));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_RESPONSE_BYTES {
                return Err(size_error(body.len()));
            }
        }
        decode_answers(&body, kind)
    }
}

impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let this = self.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let ips = this
                .lookup(&host)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            let addrs: Addrs = Box::new(
                ips.into_iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use hickory_resolver::proto::op::MessageType;
    use hickory_resolver::proto::rr::Record;

    use super::*;

    #[test]
    fn disabled_and_unknown_ids_have_no_provider() {
        for id in [-1, 0, 14] {
            assert_eq!(DohProvider::from_pref(id), None, "id {id}");
        }
    }

    #[test]
    fn the_mullvad_url_has_no_leading_space() {
        let url = DohProvider::Mullvad.url();
        assert_eq!(url, "https://dns.mullvad.net/dns-query");
        assert!(Url::parse(url).is_ok());
    }

    #[test]
    fn every_bootstrap_entry_parses_as_an_address() {
        for provider in DohProvider::ALL {
            assert_eq!(
                provider.bootstrap_addrs().len(),
                provider.bootstrap().len(),
                "{provider:?}"
            );
            assert!(provider.bootstrap_addrs().iter().all(|s| s.port() == 443));
        }
    }

    #[test]
    fn the_table_matches_the_prefs_endpoints() {
        for provider in DohProvider::ALL {
            assert_eq!(
                kuro_prefs::network::doh_endpoint(provider.id()),
                Some(provider.url()),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn private_names_are_refused_and_public_names_and_addresses_pass() {
        assert!(!is_public_name("localhost"));
        assert!(!is_public_name("nas"));
        assert!(is_public_name("example.com"));
        assert!(is_public_name("192.168.1.1"));
    }

    #[test]
    fn the_query_is_base64url_without_padding() {
        let wire = encode_query("example.com", RecordType::A).unwrap();
        let param = dns_parameter(&wire);
        assert!(!param.contains('='), "no padding: {param}");
        assert!(
            !param.contains('+') && !param.contains('/'),
            "url-safe alphabet: {param}"
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(&param).unwrap(), wire);
        let msg = Message::from_vec(&wire).unwrap();
        assert_eq!(msg.metadata.id, 0);
        assert_eq!(msg.queries[0].query_type(), RecordType::A);
    }

    fn answer_wire(answers: &[(RecordType, &str)], code: ResponseCode) -> Vec<u8> {
        let mut msg = Message::query();
        msg.metadata.message_type = MessageType::Response;
        msg.metadata.response_code = code;
        for (kind, text) in answers {
            let name = DnsName::from_str("example.com").unwrap();
            let data = match kind {
                RecordType::A => RData::A(A(text.parse().unwrap())),
                _ => RData::AAAA(AAAA(text.parse().unwrap())),
            };
            msg.add_answer(Record::from_rdata(name, 60, data));
        }
        msg.to_vec().unwrap()
    }

    #[test]
    fn answers_are_read_by_type_only() {
        let wire = answer_wire(
            &[
                (RecordType::A, "192.0.2.1"),
                (RecordType::AAAA, "2001:db8::1"),
            ],
            ResponseCode::NoError,
        );
        assert_eq!(
            decode_answers(&wire, RecordType::A).unwrap(),
            vec!["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(
            decode_answers(&wire, RecordType::AAAA).unwrap(),
            vec!["2001:db8::1".parse::<IpAddr>().unwrap()]
        );
    }

    #[test]
    fn a_non_zero_response_code_is_a_failure() {
        let wire = answer_wire(&[], ResponseCode::NXDomain);
        assert!(
            decode_answers(&wire, RecordType::A)
                .unwrap_err()
                .contains("response code")
        );
    }

    #[test]
    fn the_size_message_names_the_limit_and_the_size() {
        assert_eq!(
            size_error(65_537),
            "response size exceeds limit (65536 bytes): 65537 bytes"
        );
    }

    #[test]
    fn a_lookup_of_a_private_name_fails_without_a_request() {
        let client = reqwest::Client::new();
        let resolver = DohResolver::from_parts(
            Url::parse("https://127.0.0.1:1/dns-query").unwrap(),
            client,
            true,
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = runtime.block_on(resolver.lookup("localhost")).unwrap_err();
        assert_eq!(err.to_string(), "private hosts not resolved");
    }
}
