//! [`fetch`]: the only HTTP request path in the workspace.

use std::fmt;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use url::{Host, Url};

use crate::pinned::PinnedResolver;
use crate::reference::resolve_reference;
use crate::vet;

/// Limits for one [`fetch`] call or one [`load`](crate::load). Defaults: 4 MiB, 10 s,
/// 5 redirects, loopback off, 256 requests.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FetchPolicy {
    /// A body of exactly this length is accepted; one byte more is [`FetchError::TooLarge`].
    pub max_body_bytes: u64,
    /// One deadline for the whole call: every hop, connect, TLS, headers and body.
    pub timeout: Duration,
    /// Redirects followed per call.
    pub max_redirects: u8,
    /// Allows loopback addresses, and `http` only when every address of the hop is loopback.
    pub permit_loopback_http: bool,
    /// HTTP requests one [`load`](crate::load) may send, redirect hops included. Read only by
    /// `load`; a single [`fetch`] is bounded by `1 + max_redirects`.
    pub max_requests: u32,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        FetchPolicy {
            max_body_bytes: 4 * 1024 * 1024,
            timeout: Duration::from_secs(10),
            max_redirects: 5,
            permit_loopback_http: false,
            max_requests: 256,
        }
    }
}

/// An HTTP response received by [`fetch`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fetched {
    /// The HTTP status code of the final hop.
    pub status: u16,
    /// The raw `Content-Type` value.
    pub content_type: Option<String>,
    /// The response body, at most [`FetchPolicy::max_body_bytes`] long.
    pub body: Vec<u8>,
    url: Option<String>,
}

impl Fetched {
    /// Builds a response that was not received over HTTP, with no final URL.
    pub fn new(status: u16, content_type: Option<String>, body: Vec<u8>) -> Self {
        Fetched {
            status,
            content_type,
            body,
            url: None,
        }
    }

    /// Returns the final hop's URL, the base for relative references in the body.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

/// Why [`fetch`] refused or failed a request.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FetchError {
    /// The URL, or a redirect `Location`, is not a usable absolute URL.
    InvalidUrl,
    /// The scheme is not `https`, or is `http` while [`FetchPolicy::permit_loopback_http`] is off
    /// or an address of the hop is not loopback.
    Scheme,
    /// The host did not resolve to any address.
    Resolve,
    /// The host resolved to an address that is not globally reachable.
    DisallowedAddress(IpAddr),
    /// More than [`FetchPolicy::max_redirects`] redirects.
    TooManyRedirects,
    /// The body exceeds [`FetchPolicy::max_body_bytes`].
    TooLarge,
    /// The deadline [`FetchPolicy::timeout`] passed.
    Timeout,
    /// The response has a `Content-Encoding` other than `identity`.
    ContentEncoding,
    /// The response has several `Content-Type` headers, or one that is not visible ASCII.
    ContentType,
    /// Any other transport failure; the cause is the error source.
    Transport(Arc<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::InvalidUrl => f.write_str("invalid URL"),
            FetchError::Scheme => f.write_str("scheme not permitted"),
            FetchError::Resolve => f.write_str("host did not resolve"),
            FetchError::DisallowedAddress(ip) => write!(f, "address {ip} is not permitted"),
            FetchError::TooManyRedirects => f.write_str("too many redirects"),
            FetchError::TooLarge => f.write_str("response body too large"),
            FetchError::Timeout => f.write_str("request timed out"),
            FetchError::ContentEncoding => f.write_str("content encoding refused"),
            FetchError::ContentType => f.write_str("invalid Content-Type"),
            FetchError::Transport(e) => write!(f, "transport error: {e}"),
        }
    }
}

impl std::error::Error for FetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FetchError::Transport(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

/// Sends one GET under Safe Fetching, following redirects itself.
///
/// Every hop is checked before a request is sent: the scheme must be `https` (or loopback
/// `http` when [`FetchPolicy::permit_loopback_http`] is set) and every resolved address must be
/// globally reachable (or loopback, under the same flag). The connection is made to exactly the vetted addresses, and the
/// `Accept-Encoding` is `identity`. The status is not interpreted; see
/// [`accept_response`](crate::accept_response).
///
/// Blocking: call through `tokio::task::spawn_blocking` from async code.
///
/// # Errors
///
/// Returns the [`FetchError`] variant that names the first rule the request or response broke:
/// [`InvalidUrl`](FetchError::InvalidUrl), [`Scheme`](FetchError::Scheme),
/// [`Resolve`](FetchError::Resolve), [`DisallowedAddress`](FetchError::DisallowedAddress),
/// [`TooManyRedirects`](FetchError::TooManyRedirects), [`TooLarge`](FetchError::TooLarge),
/// [`Timeout`](FetchError::Timeout), [`ContentEncoding`](FetchError::ContentEncoding),
/// [`ContentType`](FetchError::ContentType) or [`Transport`](FetchError::Transport).
///
/// # Examples
///
/// A private address is refused before any request is sent, so this runs offline:
///
/// ```
/// use turul_ai_catalog_client::{FetchError, FetchPolicy, fetch};
///
/// let refused = fetch("https://10.0.0.1/ai-catalog.json", &FetchPolicy::default());
/// assert!(matches!(refused, Err(FetchError::DisallowedAddress(_))));
/// ```
pub fn fetch(url: &str, policy: &FetchPolicy) -> Result<Fetched, FetchError> {
    match hops(url, policy, None) {
        Ok(fetched) => Ok(fetched),
        Err(Stop::Refused(e)) => Err(e),
        Err(Stop::Budget) => unreachable!("fetch passes no budget"),
    }
}

/// The requests one [`load`](crate::load) may still send.
pub(crate) struct RequestBudget {
    remaining: u32,
}

impl RequestBudget {
    pub(crate) fn new(limit: u32) -> Self {
        RequestBudget { remaining: limit }
    }

    pub(crate) fn is_spent(&self) -> bool {
        self.remaining == 0
    }
}

pub(crate) enum Stop {
    Refused(FetchError),
    Budget,
}

impl From<FetchError> for Stop {
    fn from(e: FetchError) -> Self {
        Stop::Refused(e)
    }
}

/// The hop loop shared by `fetch` and `load`. With a budget, each request is counted just before it is sent, after the
/// hop's scheme check and address vetting.
pub(crate) fn hops(
    url: &str,
    policy: &FetchPolicy,
    mut budget: Option<&mut RequestBudget>,
) -> Result<Fetched, Stop> {
    let deadline = Instant::now() + policy.timeout;
    let mut current = url.to_string();
    let mut redirects = 0u8;
    loop {
        let parsed = Url::parse(&current).map_err(|_| FetchError::InvalidUrl)?;
        let http = match parsed.scheme() {
            "https" => false,
            "http" if policy.permit_loopback_http => true,
            _ => return Err(FetchError::Scheme.into()),
        };
        let port = parsed
            .port_or_known_default()
            .ok_or(FetchError::InvalidUrl)?;
        let (host, addrs): (String, Vec<SocketAddr>) =
            match parsed.host().ok_or(FetchError::InvalidUrl)? {
                Host::Ipv4(ip) => (String::new(), vec![SocketAddr::new(ip.into(), port)]),
                Host::Ipv6(ip) => (String::new(), vec![SocketAddr::new(ip.into(), port)]),
                Host::Domain(d) => {
                    let found: Vec<SocketAddr> = (d, port)
                        .to_socket_addrs()
                        .map_err(|_| FetchError::Resolve)?
                        .collect();
                    (d.to_string(), found)
                }
            };
        if addrs.is_empty() {
            return Err(FetchError::Resolve.into());
        }
        let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
        if let Some(bad) = vet::first_disallowed(&ips, policy) {
            return Err(FetchError::DisallowedAddress(bad).into());
        }
        if http && !ips.iter().all(IpAddr::is_loopback) {
            return Err(FetchError::Scheme.into());
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(FetchError::Timeout.into());
        }
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(remaining)
            .min_tls_version(reqwest::tls::Version::TLS_1_2);
        if !host.is_empty() {
            builder = builder.dns_resolver(Arc::new(PinnedResolver::new(&host, addrs)));
        }
        let client = builder.build().map_err(|e| transport(e, deadline))?;
        if let Some(b) = budget.as_deref_mut() {
            if b.is_spent() {
                return Err(Stop::Budget);
            }
            b.remaining -= 1;
        }
        let response = client
            .get(parsed.as_str())
            .header(ACCEPT_ENCODING, "identity")
            .timeout(remaining)
            .send()
            .map_err(|e| transport(e, deadline))?;

        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.headers().get(LOCATION)
        {
            if redirects >= policy.max_redirects {
                return Err(FetchError::TooManyRedirects.into());
            }
            let location = location.to_str().map_err(|_| FetchError::InvalidUrl)?;
            current = resolve_reference(Some(parsed.as_str()), location)
                .map_err(|_| FetchError::InvalidUrl)?;
            redirects += 1;
            continue;
        }

        let headers = response.headers();
        if headers.get_all(CONTENT_ENCODING).iter().any(|v| {
            !v.to_str()
                .is_ok_and(|s| s.trim().eq_ignore_ascii_case("identity"))
        }) {
            return Err(FetchError::ContentEncoding.into());
        }
        let mut types = headers.get_all(CONTENT_TYPE).iter();
        let content_type = match (types.next(), types.next()) {
            (None, _) => None,
            (Some(v), None) => Some(v.to_str().map_err(|_| FetchError::ContentType)?.to_string()),
            _ => return Err(FetchError::ContentType.into()),
        };
        if response
            .content_length()
            .is_some_and(|n| n > policy.max_body_bytes)
        {
            return Err(FetchError::TooLarge.into());
        }
        let mut body = Vec::new();
        response
            .take(policy.max_body_bytes.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut || Instant::now() >= deadline {
                    FetchError::Timeout
                } else {
                    FetchError::Transport(Arc::new(e))
                }
            })?;
        if body.len() as u64 > policy.max_body_bytes {
            return Err(FetchError::TooLarge.into());
        }
        return Ok(Fetched {
            status,
            content_type,
            body,
            url: Some(parsed.into()),
        });
    }
}

fn transport(e: reqwest::Error, deadline: Instant) -> FetchError {
    if e.is_timeout() || Instant::now() >= deadline {
        FetchError::Timeout
    } else {
        FetchError::Transport(Arc::new(e))
    }
}
