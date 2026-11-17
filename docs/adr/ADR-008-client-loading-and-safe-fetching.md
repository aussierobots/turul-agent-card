# ADR-008: Client loading and Safe Fetching

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-client/tests/`: `safe_fetching.rs`, `well_known.rs`, `response_acceptance.rs`, with the loopback server `support/loopback.rs` (§7). Unit tests on crate-private code live in `src/` (`vet::tests`, `pinned::tests`). The traversal half of AMB-73 is pinned in ADR-009's `nested_traversal.rs`.
- **Depends on:** ADR-002 (`parse`, `ParseError`, the strict reader), ADR-004 (`MEDIA_TYPE`, `WELL_KNOWN_PATH`), ADR-005 (`validate`, `Report`). Binding decisions: M3 (the client owns all consumer logic; well-known lookup granted, link-relation discovery not), M4 (client ↔ servers interop over loopback only), M10 (`fetch` is the request path for DID documents too), D15 (`reqwest` blocking with rustls, `url`), D16 (`FetchPolicy::permit_loopback_http`). Used by ADR-009 (traversal), ADR-010 (interop, live probe), ADR-013 (`verify_artifact`), ADR-014 (`did_web::resolve`).
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10). RFC 3986, RFC 9110, RFC 6890 and the IANA IPv4/IPv6 Special-Purpose Address Registries.

## Context

M3 puts every network operation of the workspace in `turul-ai-catalog-client`, and REV-10 makes one function, `client::fetch`, the only request path for catalogs, DID documents (ADR-014) and artifacts (ADR-013). The trust crate never opens a connection itself (PLAN §2). Every fetched input, a nested catalog `url` included, can be controlled by an attacker. So Safe Fetching (#safe-fetching) applies to every request `fetch` makes, not only those made during verification: the spec addresses "Consumers performing verification", but one request path cannot apply two policies, and a catalog `url` is the same SSRF surface.

This ADR fixes the request (`FetchPolicy`, `fetch`, `Fetched`), the one URL-reference resolver (`resolve_reference`), the catalog sources (`Source`), the one catalog response gate (`accept_response`) and the fetch half of `load`. ADR-009 owns traversal, the resolved tree, filtering, display text and JSON output.

Spec anchors implemented: #safe-fetching, #verification-procedures (introduction: why these fetches are hostile), #well-known-uri (client half), #location-independence (a catalog at any URL or in a file), #link-relation-discovery (step 4 only: media type and `specVersion` of the retrieved document; steps 1 to 3 are not granted by M3), #catalog-entry (`digest`: "hash the exact retrieved artifact bytes", AMB-11), #media-type-registration-application-ai-catalog-json (no parameters; UTF-8 JSON), #trust-layers (Layer 0: HTTPS).

## Decision

### 1. `FetchPolicy` (AMB-76, D16)

`FetchPolicy` is `#[non_exhaustive]` with public fields and `Default`:

| Field | Default | Meaning |
|---|---|---|
| `max_body_bytes: u64` | 4 194 304 (4 MiB) | A body of exactly this length is accepted; one byte more is `TooLarge`. |
| `timeout: Duration` | 10 s | One deadline for the whole `fetch` call: every hop, connect, TLS, headers and body (AMB-183). |
| `max_redirects: u8` | 5 | Redirects followed per call; one more `Location` is `TooManyRedirects`. |
| `max_requests: u32` | 256 | M16, owned by ADR-009 §2c: HTTP requests per `load`, redirects included. `fetch` alone does not read it. |
| `permit_loopback_http: bool` | `false` | D16. When `true`, loopback addresses (127.0.0.0/8, `::1`) are allowed, and `http` is allowed only when every address of the hop is loopback. Every other rule is unchanged. |

The spec leaves limits to the consumer ("apply a consumer-defined ceiling"). The defaults are documented here and in the rustdoc of `FetchPolicy`. The spec's per-fetch ceiling `attestation.size` has no caller: no code in the workspace fetches an attestation (stale-attestations-2/3 are out of scope, and ADR-009 pins that `attestation.uri` is never requested), so `fetch` takes no per-call ceiling.

### 2. `fetch(url: &str, policy: &FetchPolicy) -> Result<Fetched, FetchError>`

`fetch` makes one GET, following redirects itself. Per hop, in this order:

1. Parse the URL with the `url` crate (WHATWG). A failure is `InvalidUrl`. The scheme must be `https`. `http` is allowed only under `permit_loopback_http` and is checked again after step 3. Any other scheme, `data:` included, is `Scheme` (AMB-81 decides `data:` entry URLs in ADR-013).
2. Find the hop's addresses. For an IP-literal host, the literal is the only address, after WHATWG normalisation, so `https://2130706433/` is 127.0.0.1. For a name, resolve it once with the system resolver (`ToSocketAddrs`). No address is `Resolve`.
3. Vet every address (§3). If any address is disallowed, the hop is refused with `DisallowedAddress` and no connection is made (AMB-181).
4. Connect only to the vetted addresses. The hop's `reqwest` client is built with automatic redirects off, proxies off (AMB-182), rustls with certificate verification on and TLS 1.2 as the minimum (#trust-layers Layer 0), and a pinned resolver (crate-private `pinned::PinnedResolver`). The pinned resolver returns exactly the vetted addresses for the hop's host and an error for any other name, so `reqwest` never performs a second lookup and the name cannot be rebound between vetting and connecting.
5. Send `Accept-Encoding: identity` and no cookie or credential header. `Accept` is either absent or `*/*`, which RFC 9110 §12.5.1 makes equivalent: D15's `reqwest` always sends `*/*` and has no API to omit it, and the spec defines no content negotiation for catalogs. `fetch` is generic: acceptance is decided on the response (§5, ADR-014's `accept_document`, ADR-013).
6. A response with status 301, 302, 303, 307 or 308 and a `Location` header is a redirect. `Location` is resolved against the current hop's URL with `resolve_reference` (§4), and the result becomes the next hop, which runs steps 1 to 6 again. Any other status is a final response (AMB-184).
7. On the final response: a `Content-Encoding` other than absent or `identity` is `ContentEncoding`, so the body is the exact bytes delivered (AMB-11). More than one `Content-Type` field, or a value that is not visible ASCII, is `ContentType`. A `Content-Length` above `max_body_bytes` is `TooLarge` before the body is read. Otherwise at most `max_body_bytes + 1` bytes are read, and reading more than `max_body_bytes` is `TooLarge`. When the deadline passes at any point, the result is `Timeout`. The time left before the deadline is set as the request's own timeout, which bounds the body read as a whole; a client-level timeout alone bounds each read call separately, so a body sent one byte at a time would never time out.

`Fetched` is `#[non_exhaustive]` with public `status: u16`, `content_type: Option<String>` (the raw header value) and `body: Vec<u8>`, plus `url() -> Option<&str>`. `fetch` sets the URL to the final hop's URL, which is the base for relative references in the body (RFC 3986 §5.1.3: the URI "used to retrieve the representation"). `Fetched::new(status, content_type, body)` builds a value with `url()` `None`. It is the seam ADR-010 and ADR-014 use to test their gates without a network.

`FetchError` (`#[non_exhaustive]`, `Display`, `std::error::Error`) has one variant per refusal: `InvalidUrl`, `Scheme`, `Resolve`, `DisallowedAddress(IpAddr)`, `TooManyRedirects`, `TooLarge`, `Timeout`, `ContentEncoding`, `ContentType` and `Transport` (connect, TLS and I/O failures, with the source error).

`fetch`, and `load` built on it, are blocking (`reqwest::blocking`, D15), and `reqwest::blocking` panics when called on a tokio runtime thread. The rustdoc of each says to call it through `tokio::task::spawn_blocking` from async code (PLAN §2). No async variant exists, because no consumer needs one.

### 3. The address rule (safe-fetching-1, AMB-180)

The crate-private `vet::is_allowed(IpAddr, &FetchPolicy) -> bool` is the one address rule. It refuses every address that is not globally reachable. The spec lists examples ("e.g., `127.0.0.0/8`, `::1`, …"); the full set below is the IANA Special-Purpose Address Registries' blocks that are not globally reachable, taken whole:

- IPv4: 0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8 (unless `permit_loopback_http`), 169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24, 192.0.2.0/24, 192.88.99.0/24, 192.168.0.0/16, 198.18.0.0/15, 198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4 and 240.0.0.0/4 (which includes 255.255.255.255). Two /32s inside 192.0.0.0/24 are globally reachable in the registry and are refused anyway: allowing them would only widen the rule.
- IPv6: an address in `::ffff:0:0/96` (IPv4-mapped) or `64:ff9b::/96` (NAT64, RFC 6052) is vetted as its embedded IPv4 address, so `[::ffff:127.0.0.1]` is loopback, and a DNS64 network still reaches public IPv4 hosts. Otherwise only 2000::/3 is allowed, minus 2001::/23, 2001:db8::/32, 2002::/16 (6to4, which embeds an IPv4 address that is not checked) and 3fff::/20. So `::`, `::1` (unless `permit_loopback_http`), `fc00::/7`, `fe80::/10`, `fec0::/10`, `ff00::/8`, `100::/64` and `64:ff9b:1::/48` are refused.

"Cloud metadata addresses" need no separate list, because each known one falls in a refused block: 169.254.169.254 (AWS, Azure, GCP), `fd00:ec2::254` (AWS IPv6, in `fc00::/7`), 100.100.100.200 (Alibaba, in 100.64.0.0/10) and 192.0.0.192 (Oracle, in 192.0.0.0/24). A metadata hostname such as `metadata.google.internal` is refused by its resolved address.

### 4. `resolve_reference(base: Option<&str>, reference: &str) -> Result<String, Unresolvable>` (AMB-73)

`resolve_reference` is the one URL-reference resolver in the workspace. Its callers are `fetch` (`Location`), `load` (nested `url`, ADR-009) and trust `verify_artifact` (ADR-013). If `reference` parses as an absolute URL, the result is that URL serialised. Otherwise, with `Some(base)`, the result is `Url::parse(base)?.join(reference)` serialised. With `None`, or when either parse fails, the result is `Unresolvable`.

The algorithm is the `url` crate's WHATWG resolution, not a hand-written RFC 3986 §5.2. `reqwest` requests a `url::Url`, so a second resolver whose output can differ for abnormal input (backslashes, surrounding whitespace, special-scheme normalisation) would make the URL that is compared differ from the URL that is fetched. For the RFC 3986 §5.4.1 normal examples against an `http` base the two agree, and the test pins exactly that set. The serialised string is the key ADR-009 uses for cycles, compared exactly with no further normalisation (AMB-73).

`resolve_reference` lands in S8 with its first caller, the `Location` header, not in S9 as PLAN S9 lists (plan drift, §Consequences).

### 5. Sources and the catalog response gate (AMB-52, AMB-56, AMB-74)

`Source` (`#[non_exhaustive]`) is `File(PathBuf)`, `Url(String)` or `WellKnown(WellKnownHost)`.

- `WellKnownHost::new(&str) -> Result<WellKnownHost, InvalidHost>` accepts a host with an optional `:port` and nothing else: `https://` + host + `WELL_KNOWN_PATH` must parse, with no userinfo, query or fragment, and with a serialised path exactly `WELL_KNOWN_PATH` (AMB-185). `Source::request_url(&self) -> Option<String>` is the one place the well-known URL is built (`None` for `File`). `load` calls it, and the CLI can print it.
- `Source::WellKnown` is the only source that requests the well-known URI. `Url` fetches exactly the URL given. Neither falls back to the other, and no Link header or HTML is ever read (AMB-56; RD-28, RD-29, RD-32).
- `Source::File` is read whole with `std::fs`. It has no media type, so no media-type rule applies (AMB-52), and no size ceiling applies: #safe-fetching governs fetches, and a file is named by the local user. Its base is `None`.

`accept_response(&Fetched) -> Result<Accepted, AcceptError>` is the one catalog response gate. It is used by `load` for every HTTP catalog, root and nested, and by ADR-010's interop test. In order:

1. `status` must be exactly 200; anything else is `AcceptError::Status(u16)`. Redirects are consumed by `fetch`. A 2xx other than 200 does not carry the requested representation as a complete body (204, 206) or may have been transformed by a proxy (203) (AMB-184).
2. Media type. The `Content-Type` value is split at the first `;`, trimmed of whitespace, and compared ASCII case-insensitively (RFC 9110 §8.3.1). Parameters are ignored: the registration defines none, and the body must be UTF-8 whatever a `charset` says (ADR-002 reader). `application/ai-catalog+json` (`MEDIA_TYPE`) is accepted. `application/json` is accepted with the warning `LoadWarning::MediaTypeGeneric`, code `media-type-generic` (#link-relation-discovery step 4: "a generic type such as `application/json` is acceptable", AMB-74). A missing `Content-Type` and every other value (`text/html`, `text/json`, `application/ld+json`) are `AcceptError::MediaType(Option<String>)`. Step 4 asks for "a JSON media type", and the spec names only these two (AMB-186).
3. `parse(body)`: the strict reader, then the `specVersion` gate (ADR-002), which is step 4's "contains a valid `specVersion` field". A failure is `AcceptError::Parse(ParseError)`.

`Accepted { catalog: AiCatalog, warnings: Vec<LoadWarning> }`. `LoadWarning` (`#[non_exhaustive]`) is the client's vocabulary for retrieval outcomes. It is not a document diagnostic: `Report` (ADR-005) describes a document, and the media type is a property of one response. `media-type-generic` is its only code, and ADR-005 does not define that code.

### 6. `load` (fetch half)

`load(&Source, &FetchPolicy) -> Result<ResolvedCatalog, LoadError>`:

- `File`: read, then `parse`, then `validate`. `LoadError::Io` or `LoadError::Parse`.
- `Url`, `WellKnown`: `fetch(request_url)`, then `accept_response`, then `validate`. `LoadError::Fetch(FetchError)` or `LoadError::Accept(AcceptError)`.
- The root `ResolvedCatalog` carries `document`, `report`, `base` (`Fetched::url()`, `None` for a file) and `warnings`. ADR-009 adds traversal, entry rows and children (S9).

Every fetched document is untrusted input (safe-fetching-4). Bytes reach the model only through the strict reader (ADR-002: UTF-8 only, depth limit 128, duplicate members refused). The body is never interpreted before that, and strings reach front ends only through `DisplayText` (ADR-009). A hostile body is a `LoadError`, never a panic.

### 7. Test seam (M4)

`tests/support/loopback.rs` is the one loopback HTTP/1.1 server for client tests. It uses `std::net`, binds `127.0.0.1:0`, scripts each response (status, headers, body, delays, a stall after the headers), and logs every request line and header. Each client test file includes it with `mod support;`. Loopback tests run with `permit_loopback_http`. Refusals with the default policy need no server: they fail before connecting.

Named gaps, not new controls. `Source::WellKnown` is `https` only, so its network half never runs in CI: `request_url` is tested by string, and the request itself is `fetch`, tested over loopback. TLS and certificate verification are not exercised by loopback tests.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| A request and its body | `client::fetch` | itself; called by `load`, trust `did_web::resolve`, trust `digest::verify_artifact` |
| Limits | `client::FetchPolicy` (`Default` per §1) | every `fetch` call |
| Address rule | crate-private `vet::is_allowed` | `fetch`, per hop |
| Connect-to-vetted | crate-private `pinned::PinnedResolver` | `fetch`, per hop |
| Resolved URL string (cycle key, redirect target, artifact URL) | `client::resolve_reference` | `fetch`, `load`, trust `verify_artifact` |
| Well-known URL | `Source::request_url` over `WellKnownHost` and `identity::WELL_KNOWN_PATH` | `load` |
| Catalog response gate | `client::accept_response` → `Accepted`, `AcceptError` | `load`; lambda `consumer_interop` |
| Retrieval warning vocabulary | `client::LoadWarning` (`media-type-generic`; `request-budget` is owned by ADR-009 §2c) | `accept_response`; `load` for `request-budget` |
| Root catalog from a source | `client::load` (fetch half) | itself |
| Test server | `crates/turul-ai-catalog-client/tests/support/loopback.rs` | every client test file |

No derived artifact, golden bytes or schema digest belongs to this ADR (D17 applies to ADR-003, ADR-009 and ADR-013).

## Requirement → test

Test paths are under `crates/turul-ai-catalog-client/tests/` unless they name `src/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| safe-fetching-1 | "Resolve and reject URLs targeting private, loopback, link-local, or cloud metadata addresses … and re-check the resolved address after any redirect" | `safe_fetching::disallowed_literal_addresses_are_refused_before_connecting` (`https://127.0.0.1/`, `https://10.0.0.1/`, `https://[::1]/`, `https://169.254.169.254/`, `https://[fc00::1]/`, `https://[::ffff:127.0.0.1]/`, `https://2130706433/`, `https://0x7f.0.0.1/`, `https://0.0.0.0/`, `https://[::]/`, `https://100.100.100.200/`, `https://192.0.0.192/`, `https://[fd00:ec2::254]/`), `safe_fetching::hostname_resolving_to_loopback_is_refused` (`https://localhost/`, through the system resolver), `safe_fetching::redirect_to_a_disallowed_address_is_refused` (loopback 302 to `https://169.254.169.254/`, under `permit_loopback_http`), `src/vet.rs` `vet::tests::special_purpose_blocks_are_refused`, `vet::tests::global_unicast_is_allowed`, `vet::tests::embedded_ipv4_is_vetted_as_ipv4`, `vet::tests::resolution_mixing_public_and_private_is_refused` |
| safe-fetching-2 | "Restrict fetches to the `https` scheme … and refuse to follow redirects that cross into a disallowed address range" | `safe_fetching::non_https_schemes_are_refused` (`http://`, `data:`, `file:`, `ftp:`), `safe_fetching::redirect_to_http_is_refused` (loopback 302 to `http://` on a public name), `safe_fetching::redirect_to_a_disallowed_address_is_refused` |
| safe-fetching-3 | "Enforce a maximum response size and a request timeout … apply a consumer-defined ceiling" | `safe_fetching::default_policy_limits` (4 MiB, 10 s, 5 redirects, loopback off), `safe_fetching::body_at_the_ceiling_is_accepted`, `safe_fetching::body_over_the_ceiling_is_refused` (with and without `Content-Length`), `safe_fetching::stalled_response_times_out`, `safe_fetching::trickling_body_times_out`, `safe_fetching::deadline_spans_redirect_hops`, `safe_fetching::sixth_redirect_is_refused` |
| safe-fetching-4 | "Treat every fetched document as untrusted input" | `safe_fetching::fetched_documents_are_untrusted_input` (129 nested arrays give `Accept(Parse(DepthLimit))`, invalid UTF-8 gives `Accept(Parse(InvalidUtf8))`, a duplicate member gives `Accept(Parse(DuplicateMember))`; none panics) |
| well-known-uri-2 | "Clients performing domain-level discovery SHOULD attempt to retrieve this well-known URL" | `well_known::well_known_source_requests_the_well_known_path` (`WellKnownHost::new("h")` gives `https://h/.well-known/ai-catalog.json`; `h:8443` keeps its port), `well_known::invalid_well_known_hosts_are_refused` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-19 (fetch half) | every fetch goes through `client::fetch` | `safe_fetching::fetch_is_the_only_request_path` (source scan: `reqwest` is named only in `src/fetch.rs` and `src/pinned.rs`; the trust crate's `src/` names neither `reqwest` nor `std::net`) |
| RD-30 | media type checked | `response_acceptance::ai_catalog_media_type_is_accepted`, `response_acceptance::other_media_types_are_refused` |
| RD-31 | `specVersion` gated | `response_acceptance::unsupported_major_is_refused` (`{"specVersion":"2.0"}` gives `Accept(Parse(UnsupportedMajor))`) |
| RD-32 | well-known only when asked | `well_known::well_known_is_requested_only_for_a_well_known_source` (a `Source::Url` load leaves no well-known request in the loopback log, even after a redirect) |
| RD-34 | Safe Fetching enforced | every `safe_fetching` function above |
| D16 | loopback `http` only under the switch | `safe_fetching::loopback_http_is_refused_by_default`, `safe_fetching::loopback_http_is_permitted_only_by_the_switch` (`http://127.0.0.1:p/` is served; with the switch on, `http://` to a public name is still `Scheme`) |
| AMB-11 (fetch half; the digest half is ADR-013) | identity encoding requested; any other `Content-Encoding` refused | `safe_fetching::identity_encoding_is_requested_and_other_encodings_refused` (the log shows `Accept-Encoding: identity`; `gzip` gives `ContentEncoding`) |
| AMB-52 (media-type half) | a file has no media type | `response_acceptance::file_source_has_no_media_type_rule` |
| AMB-56 (client half; server half ADR-010) | well-known requested only for `Source::WellKnown` | `well_known::well_known_is_requested_only_for_a_well_known_source` |
| AMB-73 (resolution; the cycle-key half is ADR-009) | one WHATWG resolver; no base gives `Unresolvable` | `safe_fetching::resolve_reference_matches_rfc_3986_normal_examples`, `safe_fetching::reference_without_base_is_unresolvable_unless_absolute`, `safe_fetching::relative_redirect_location_resolves_against_the_hop_url` |
| AMB-74 | `application/ai-catalog+json` accepted; `application/json` with `media-type-generic`; anything else refused | `response_acceptance::ai_catalog_media_type_is_accepted`, `response_acceptance::generic_json_gives_media_type_generic`, `response_acceptance::other_media_types_are_refused` |
| AMB-76 | default limits documented | `safe_fetching::default_policy_limits` |
| AMB-180 | address rule (§3) | the four `vet::tests` functions, `safe_fetching::disallowed_literal_addresses_are_refused_before_connecting` |
| AMB-181 | one disallowed address refuses the hop | `vet::tests::resolution_mixing_public_and_private_is_refused` |
| AMB-182 | proxies never used | `safe_fetching::proxy_environment_is_ignored` (the test re-runs itself in a child process with `ALL_PROXY`, `HTTP_PROXY` and `HTTPS_PROXY` pointing at a second loopback listener, which must log nothing) |
| AMB-183 | one deadline per call | `safe_fetching::deadline_spans_redirect_hops` (three hops of 0.4 s under a 1 s timeout give `Timeout`), `safe_fetching::trickling_body_times_out` (20 body bytes, one per 0.3 s, under a 1 s timeout give `Timeout` within 2 s) |
| AMB-184 | redirect statuses; `base` is the final URL; status exactly 200 | `safe_fetching::sixth_redirect_is_refused`, `safe_fetching::fetched_url_is_the_final_url`, `safe_fetching::redirect_without_location_is_a_final_response`, `response_acceptance::status_other_than_200_is_refused` (203, 204, 206, 302 without `Location`, 404) |
| AMB-185 | `WellKnownHost` grammar | `well_known::invalid_well_known_hosts_are_refused` (`h/x`, `h?q`, `h#f`, `u@h`, `https://h`, empty) |
| AMB-186 | media-type parsing | `response_acceptance::media_type_is_case_insensitive_and_parameters_are_ignored` (`Application/AI-Catalog+JSON; charset=utf-8` accepted), `response_acceptance::missing_or_repeated_content_type_is_refused` |
| — | pinned resolver | `src/pinned.rs` `pinned::tests::resolver_returns_exactly_the_vetted_addresses_for_the_hop_host_only` |

New ids (not in PLAN §6), AMB-180 to AMB-186. The block AMB-180 to AMB-189 is reserved for this ADR, so ids added by ADRs written concurrently cannot collide. Unused numbers stay unused.

- AMB-180: #safe-fetching names classes with examples. "Not globally reachable" in the IANA registries is the only published, complete definition of those classes, and it already contains every known metadata address. A hand-picked subset would leave gaps, for example 100.64.0.0/10, which holds Alibaba's metadata service.
- AMB-181: "reject URLs targeting" such an address. A name with one private record targets it. Filtering out the bad record and connecting to the rest would let an attacker's DNS keep a private record in reserve.
- AMB-182: with a proxy, the proxy resolves and connects, so vetting the hop's own addresses checks nothing. `reqwest` reads proxy environment variables by default.
- AMB-183: "a request timeout". To the caller, one `fetch` is one request. Per-hop timeouts would allow (1 + `max_redirects`) × `timeout`.
- AMB-184: RFC 9110 §15.4 defines 301, 302, 303, 307 and 308 as redirects with `Location`; 300 and 304 do not redirect a GET. RFC 3986 §5.1.3 makes the final URL the base. Exactly 200 is accepted, for the reasons in §5.
- AMB-185: RFC 8615 places a well-known URI at the root of an origin. A "host" carrying a path, query, fragment or userinfo would name another resource.
- AMB-186: §5 step 2. `text/json` and other `+json` types are refused because the spec names only `application/ai-catalog+json` and `application/json`.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust (`ai-catalog-discover`) | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-34 | `HttpFetcher` has no private-address guard; follows up to 10 `https` redirects automatically; 30 s; 10 MiB; honours proxy settings | §1 to §3: vetted and pinned per hop, `fetch` follows redirects itself, 5 redirects, 10 s, 4 MiB, no proxy | #safe-fetching MUST | `safe_fetching::*` |
| RD-30 | `as_catalog` accepts any 2xx and ignores the media type | exactly 200; AMB-74 media types | #link-relation-discovery step 4, #location-independence | `response_acceptance::status_other_than_200_is_refused`, `response_acceptance::other_media_types_are_refused` |
| RD-31 | any non-empty `specVersion` | the ADR-002 gate | #consumer-behavior | `response_acceptance::unsupported_major_is_refused` |
| RD-32 | well-known tried for every input, and again on the redirected host | only for `Source::WellKnown` | #well-known-uri SHOULD, for domain-level discovery | `well_known::well_known_is_requested_only_for_a_well_known_source` |
| RD-28, RD-29 | Direct step; HTML parsed even when a `Link` header is present | not built (M3 grants well-known lookup only) | #link-relation-discovery steps 1 to 3 | — (no code exists to test) |
| RD-19 (fetch half) | verification delegates fetching to the caller | the trust crate fetches only through `client::fetch` | #safe-fetching | `safe_fetching::fetch_is_the_only_request_path` |

## Consequences

- One request path and one URL resolver. A second `reqwest` call site, a `Url::join` outside `resolve_reference`, or a proxy-enabled client is a defect, and `fetch_is_the_only_request_path` fails on the first.
- Strictness costs, accepted: a 203 or 204 catalog, a `text/json` catalog, a compressed response, a host whose DNS also returns a private address, and a fetch through a corporate proxy all fail. Each is a named error.
- Plan drift recorded:
  - PLAN S9 lists `resolve_reference` as S9 code. It lands in S8, because `fetch`'s `Location` handling is its first caller, and one resolver serves redirects, nested `url`s and artifacts.
  - PLAN §2 lists `Fetched{status, content_type, body}`. This ADR adds `Fetched::url()`, the final hop's URL (`None` from `Fetched::new`), which `load` uses as `base`. `Fetched::new` keeps the PLAN signature used by ADR-010 and ADR-014.
  - PLAN §2 lists `Source{File, Url, WellKnown(host)}`. `WellKnown` carries `WellKnownHost`, a typed constructor for the one well-known URL, and `Source::request_url` is added.
  - PLAN S8 names `media-type-generic` as "a warning" without an owner. It is `LoadWarning::MediaTypeGeneric` in the client, not a format-crate `Code` (§5).
  - ADR-004 §"MEDIA_TYPE" mentions "the client's `Accept`". `fetch` sends no `Accept` (§2 step 5); `MEDIA_TYPE` is imported by `accept_response`. ADR-004's owner may correct that sentence.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S8: `FetchPolicy`, `fetch`, `Fetched` (with `url()`), the address rule, the pinned resolver, `resolve_reference` (moved from S9), `Source` with `WellKnownHost`, `accept_response`, `LoadWarning`, and `load` (fetch half). AMB-180 to AMB-186 added in the reserved block AMB-180 to AMB-189. |
| 2026-11-17 | §2 step 5: `Accept` may be absent or `*/*` (RFC 9110 §12.5.1, equivalent). "No `Accept`" had no spec anchor and cannot be met under D15 (`reqwest` inserts `Accept: */*` in `ClientBuilder::new` and cannot remove it). `safe_fetching::identity_encoding_is_requested_and_other_encodings_refused` migrated. |
| 2026-11-17 | Accepted at S8 close (0.0.10): `safe_fetching.rs` (22), `well_known.rs` (3) and `response_acceptance.rs` (8) are present and green, with `vet::tests` and `pinned::tests` (5); `fetch` is the only code in the workspace that sends a request (`reqwest` is used only in `fetch.rs` and `pinned.rs`), and `resolve_reference` is the only code that resolves a URL reference (`Url::join` only in `reference.rs`). |
| 2026-11-17 | M16 (owner ADR-009 §2c): `FetchPolicy` gains `max_requests` (default 256), read only by `load`; the hop loop counts each request sent against a crate-private per-`load` budget, and public `fetch` runs it unbudgeted; `LoadWarning` gains `RequestBudget` (`request-budget`); `LoadError` gains `Budget`. `safe_fetching::default_policy_limits` is unchanged; the new default is pinned in ADR-009's `nested_traversal`. |
| 2026-11-17 | `media_type_essence(&str) -> &str` made public (T3): the one parser of a `Content-Type` value's media type, used by `accept_response` and by trust `did_web::accept_document`. |
| 2026-11-17 | Deadline defect fixed: the body read was bounded only per read call (reqwest blocking applies the client timeout to each `read`), so a trickling body held `fetch` far past `timeout` (probe: 1 s timeout, 20 bytes at 0.3 s each, `Ok` after 6 s). Each hop now sets the remaining time as the request timeout, which reqwest applies to the whole body. §2 step 7 states it; `safe_fetching::trickling_body_times_out` and the drip mode of `support/loopback.rs` added. |
