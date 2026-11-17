# Handoff → turul-jwt-validator session (fig)

**Owner:** turul-jwt-validator (`~/turul-jwt-validator`). Its version, CHANGELOG and release are its own decisions.
**Consumer:** turul-agent-card, through the trust crate (`turul-ai-catalog-trust`, planned) and decision M10.
**Normative source:** AI Catalog spec, sections "Signed Payload and JWS Construction", "The did:web Signer Profile", "JWS Algorithm and Key Requirements" and "DID Document Resolution and Key Selection" (`specification/ai-catalog.md` in `github.com/Agent-Card/ai-catalog`, revision 2026-11-10). Also RFC 7515 (incl. Appendix F, detached content), RFC 7517, RFC 7518 (ES256), RFC 7797 (`b64`).

## Goal

Add a detached-JWS sign and verify API next to the existing `JwtValidator`. It must work without JWT claims, without a JWKS URL and without network access. AI Catalog signatures are detached compact JWS (`<header>..<sig>`) over caller-supplied payload bytes (JCS), checked against one caller-supplied JWK.

## What turul-agent-card needs (names are yours)

1. **Verify detached:** input is a stored `header..signature`, the payload bytes and a public JWK, plus a policy. The function rebuilds the signing input `b64url(header).b64url(payload)` and verifies it. The policy is:
   - an exact expected `typ` (`ai-catalog-trust-manifest+jws` or `ai-catalog+jws`), rejected if missing or different;
   - an `alg` allowlist (ES256 only for `did-web-v1`), with `none` always rejected;
   - reject any `b64` member;
   - reject `crit` naming any parameter that isn't understood;
   - caller-named forbidden header parameters (`jku`, `jwk`, `x5u`, `x5c` for `did-web-v1`).

   It returns the decoded protected header, so we can check that `kid` equals `signer#fragment`. It never reads `aud`, `exp` or other claims.
2. **Sign detached:** ES256 with a private key and protected header `{alg, typ, kid}`. Returns `header..signature`. Signing must be deterministic up to ECDSA randomness, and the output must pass item 1.
3. **Public JWK check:** `kty` = `EC`, `crv` = `P-256`, and `x`/`y` must be valid curve coordinates. Reject any private member (`d`). If present, `alg` must be `ES256`, `use` must be `sig`, and `key_ops` must include `verify`. A `kid` inside the JWK is ignored for selection.
4. **No forced network or async:** with `default-features = false` plus a crypto backend, the items above build without `reqwest` or `tokio`. JWKS fetching stays behind its own feature. The `aws_lc_rs` and `rust_crypto` backends both keep working.

Out of scope there (owned by turul-agent-card): JCS, `did:web` resolution and fetching, DID-document key selection, authority, coverage, time acceptance.

## Gates

- RFC 7515 Appendix A.3 (ES256) and Appendix F (detached) vectors pass on both backends.
- A negative test for each policy rule: wrong or missing `typ`, `alg: none`, `alg` outside the allowlist, `b64` present, unknown `crit`, a forbidden parameter, a JWK with `d`, the wrong curve, invalid coordinates.
- The existing `JwtValidator` behaviour and tests are unchanged.
- The no-network gate is the amended gate in the Agreed section below.
- There is a negative JWK test for each rule in item 3: `kty` other than `EC`, `crv` other than `P-256`, invalid coordinates, `d` present, `alg` other than `ES256` (e.g. ES384), `use` other than `sig` (e.g. `enc`), and `key_ops` without `verify` (e.g. `["sign"]`).

## Agreed with turul-jwt-validator (2026-11-17)

- Gate 4 is amended so existing consumers are unaffected. `rust_crypto` and `aws_lc_rs` keep their meaning and imply a default-on `jwks` feature (`JwtValidator`, `reqwest`, `tokio`). The new backend-only features `detached_rust_crypto` and `detached_aws_lc_rs` carry the detached API. The gate is that `cargo tree -e normal --no-default-features --features detached_<backend>` shows no `reqwest` or `tokio` for both backends.
- The consumer depends on it with `default-features = false, features = ["detached_rust_crypto"]`.
- The API takes the payload as `&[u8]` and the JWK as raw JSON bytes. Each policy rule has a distinct error variant. A `detached_*` feature plus its backend feature compile together.
- The point-on-curve check uses optional `aws-lc-rs` / `p256`, tied to the backend, and runs at JWK construction.
- The API, implemented in 0.3.4: `detached::{check_public_jwk, verify_detached, sign_detached_es256, DetachedJwsPolicy, DetachedJwsError}`. `verify_detached` returns the protected header as `serde_json::Map`, and the trust crate converts it at its boundary; the typed header is not requested. `typ` is compared byte-exact and case-sensitive. Any input other than `header..sig` is Malformed. Every `crit` is rejected. Duplicate header members are rejected. High-S signatures are accepted, so signature bytes are never an identifier.
- Added for M12 (the operator approved it): `inspect_detached(detached, &DetachedJwsPolicy) -> Result<Map, DetachedJwsError>` runs the same private `check_header` path as `verify_detached` (structure, header, none, allowlist, b64, crit, forbidden, typ), with no key, and stops before the signature. An `Ok` says nothing about the signature segment. `kid` is not checked; the trust crate enforces `kid` = `signer#fragment`.
- Confirmed: each of the seven JWK rules returns InvalidJwk, with a test that asserts the specific reason, on both backends.
- **Released:** 0.3.4 was published to crates.io on 2026-11-17 (checked against the crates.io API: features `detached_rust_crypto`, `detached_aws_lc_rs`, `jwks`). Slices S14 and T2–T4 are unblocked.
- The first version with the API is **0.3.4**: additive, planned, not yet released. It is referenced as `"0.3"`, and ADRs record 0.3.4 as the minimum.

## Stop conditions

- If item 4 can't be done without breaking the current public API, stop and report the options. Don't break it.
- If `jsonwebtoken` can't verify detached input or enforce the policy, report what is missing before adding another crypto dependency.
- When released, report the version (major.minor) so turul-agent-card can depend on it.
