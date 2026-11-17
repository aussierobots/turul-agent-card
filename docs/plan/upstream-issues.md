# Draft upstream issues: ai-catalog-rust 0.4.0

Drafts only. Nothing here has been posted.

- Specification: `specification/ai-catalog.md`, revision 2026-11-10. Sections are cited by heading anchor.
- Implementation: ai-catalog-rust 0.4.0. Paths are relative to `crates/`.
- Every reproduction was run against 0.4.0 with this harness. The output under each case was observed, not inferred.

```rust
let catalog = ai_catalog::parse_str(doc)?;
let result = ai_catalog_validate::validate(&catalog);
// printed: result.is_valid, result.conformance_level, error codes@paths, ai_catalog::to_string(&catalog)
```

---

## 1. Absent required members are accepted and written back as empty values (RD-01)

**Spec.** In [#top-level-structure](#top-level-structure): "An AI Catalog document is a JSON object that MUST contain the following members:" `specVersion` and `entries`. In [#host-info](#host-info): "It MUST contain: `displayName`". In [#catalog-entry](#catalog-entry): "It MUST contain the following members:" `identifier` and `type`.

**Current behaviour.**
- `ai-catalog/src/model.rs:14-19`: `#[serde(default)] pub spec_version: String` and `#[serde(default)] pub entries: Vec<CatalogEntry>`. The same applies to `HostInfo.display_name` (`:86-87`), `CatalogEntry.identifier` and `entry_type` (`:101-106`), and the required members of `Publisher`, `Subject`, `Signature`, `TrustSchema`, `Attestation` and `ProvenanceLink` (`:141-291`).
- `validate` reports an empty `specVersion`, `host.displayName`, `identifier` or `type` (`ai-catalog-validate/src/lib.rs:208`, `:263`, `:271`, `:393`). Nothing in `validate` checks `entries`, so a document without `entries` is reported valid.
- `ai_catalog::to_string` (`ai-catalog/src/lib.rs:42-44`) writes the defaulted values out as members that were never in the input.

**Reproduction.**

```json
{"specVersion":"1.0"}
```
Result: `is_valid=true`, no errors. `to_string` gives `{"specVersion":"1.0","entries":[]}`.

```json
{"specVersion":"1.0","entries":[{"url":"https://e.example/a"}]}
```
Result: `entry-member-missing` at `identifier` and `type` (correct). `to_string` gives `{"specVersion":"1.0","entries":[{"identifier":"","type":"","url":"https://e.example/a"}]}`.

**Expected.** A document without `entries` is not valid. Re-serialising does not add `""` or `[]` members that the input lacked. One way to get both is to make required members non-defaulted, so that parsing fails, or to make them `Option` and have `validate` report absence.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::missing_required_member_is_refused_by_us_and_defaulted_by_them`.

---

## 2. `"data": null` is treated as absent, and an empty `url` is treated as absent (RD-02)

**Spec.** In [#catalog-entry](#catalog-entry): "A Catalog Entry MUST contain exactly one of the following members to provide the artifact content:" `url`, "A string containing a URL…", and `data`, "A JSON value containing the complete artifact document inline." `null` is a JSON value (RFC 8259 §3).

**Current behaviour.**
- `ai-catalog/src/model.rs:109-110`: `data: Option<Value>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`. serde maps `"data": null` to `None`, and serialisation then drops the member.
- `ai-catalog-validate/src/lib.rs:279`: `match (!text(&entry.url).is_empty(), entry.data.is_some())`. Here `text` (`:478-480`) turns `Some("")` into `""`, so an empty `url` counts as absent.

**Reproduction.**

```json
{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"https://e.example/a","data":null}]}
```
Result: `is_valid=true`, even though both members are present. `to_string` drops `"data":null`.

```json
{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"","data":{"k":1}}]}
```
Result: `is_valid=true`, even though both members are present.

**Expected.** Both documents get `entry-artifact-source`, because both members are present. The first document round-trips with `"data":null`. An empty `url` is reported, because `""` is not a URL.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::null_data_is_an_artifact_source_and_their_writer_drops_it`.

---

## 3. Empty arrays are dropped on serialisation, which changes signed payloads (RD-03)

**Spec.** In [#signed-payload-and-jws-construction](#signed-payload-and-jws-construction): "All other members and values MUST be retained, including `signature.signer`, `signature.profile`, `signature.issuedAt`, optional `signature.expiresAt`, and unrecognized members." In [#independent-contributions-and-updates](#independent-contributions-and-updates): "Registries SHOULD preserve the chosen manifest and its signature rather than merge separately signed contents."

**Current behaviour.**
- `ai-catalog/src/model.rs` uses `#[serde(default, skip_serializing_if = "Vec::is_empty")]` on `CatalogEntry.tags` (`:115`), `CatalogEntry.trust_manifests` (`:122`), `TrustManifest.attestations` (`:165`), `TrustManifest.provenance` (`:167`) and `TrustSchema.verification_methods` (`:262`). A member published as `[]` is gone after `to_string`.
- Verification itself is not affected. `ai_catalog_trust::canonicalize_trust_manifest` reads the decoded JSON (`ai-catalog-trust/src/lib.rs:135-138`), and `verify` reads the raw document (`ai-catalog-validate/src/verify.rs:112`). A catalog or manifest that is parsed and written back out, however, no longer has the payload its signature covers.

**Reproduction.** The manifest below, as published, inside an entry. `jws` is structural only.

```json
{"contributor":"did:web:e.example",
 "subject":{"identifier":"urn:a","type":"application/json","digest":"sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"},
 "trustSchema":{"identifier":"s","version":"1","verificationMethods":[]},
 "attestations":[],"provenance":[],
 "signature":{"signer":"did:web:e.example","profile":"did-web-v1","issuedAt":"2026-01-01T00:00:00Z","jws":"eyJhbGciOiJFUzI1NiIsInR5cCI6ImFpLWNhdGFsb2ctdHJ1c3QtbWFuaWZlc3QrandzIn0..c2ln"}}
```
Result: `canonicalize_for_signature` of the published manifest includes `"attestations":[]`, `"provenance":[]` and `"verificationMethods":[]`. The same call on that manifest after `parse_str` and `to_string` includes none of them, so the two payloads differ. The entry's `"tags":[]` is dropped in the same way, which changes a catalog-level signature payload.

**Expected.** Parsing and then serialising keeps an empty array that was present, for example by modelling these members as `Option<Vec<_>>`. Absent and `[]` then both round-trip.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::empty_arrays_survive_our_round_trip_and_not_theirs`.

---

## 4. Duplicate member names are silently collapsed inside trust manifests, unrecognized members and JSON values (RD-04)

**Spec.** In [#signed-payload-and-jws-construction](#signed-payload-and-jws-construction): "The payload MUST satisfy JCS's I-JSON constraints; parsers MUST reject duplicate JSON member names rather than silently discard one."

**Current behaviour.**
- Duplicates of members that `AiCatalog`, `CatalogEntry`, `HostInfo` and `Publisher` model as struct fields are already rejected by `parse_str`, for example "duplicate field `specVersion`".
- `TrustManifest` deserialises through `serde_json::Value` first (`ai-catalog/src/model.rs:199-202`, `let json = Value::deserialize(deserializer)?;`). `Value` keeps the last duplicate, so every member of a manifest, including `contributor`, `subject` and `signature`, collapses silently.
- Unrecognized members (`#[serde(flatten)] extra_fields: BTreeMap`, for example `:27-28`), and the contents of `data` and `extensions`, also keep the last duplicate.
- `verify` rejects all of these, because it calls `ai_catalog_trust::parse_json` first (`ai-catalog-validate/src/verify.rs:112`). `ai_catalog::parse_*` (`ai-catalog/src/lib.rs:25-35`) followed by `validate` does not.

**Reproduction.**

```json
{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"https://e.example/a",
 "trustManifests":[{"contributor":"did:web:evil.example","contributor":"did:web:e.example","trustSchema":{"identifier":"s","version":"1"}}]}]}
```
Result: `parse_str` returns Ok and `is_valid=true`. `to_string` keeps only `"contributor":"did:web:e.example"`. `ai_catalog_trust::parse_json` rejects the same bytes: "duplicate member name 'contributor'".

```json
{"specVersion":"1.0","entries":[],"x":1,"x":2}
```
Result: `parse_str` returns Ok, and `to_string` gives `"x":2`.

**Expected.** `ai_catalog::parse_*` rejects duplicate member names at every depth, as `ai_catalog_trust::parse_json` already does, so that parsing a document has one outcome whichever entry point is used.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::duplicate_member_is_refused_by_us_and_last_wins_for_them`.

---

## 5. `ConformanceLevel::Minimal` is reported for documents that are not conformant (RD-06)

**Spec.** In [#level-1-minimal-catalog](#level-1-minimal-catalog): "A conformant Minimal Catalog is a JSON document … that contains: `specVersion` … `entries` … each containing at minimum `identifier`, `type`, and exactly one of `url` or `data`" and "Each manifest in `trustManifests`, when present at any level, MUST satisfy [Manifest Validity]." In [#conformance-levels](#conformance-levels): "Implementations MUST satisfy all requirements of their declared level."

**Current behaviour.** `ai-catalog-validate/src/lib.rs:179-185`:

```rust
if !self.errors.is_empty() || catalog.host.is_none() {
    ConformanceLevel::Minimal
```

The variant's own documentation reads "A document with errors, or without a host." (`:37`). The same holds for `Verification::conformance_level` (`verify.rs:144-150`).

**Reproduction.**

```json
{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"https://e.example/a","trustManifests":[{"contributor":"did:web:e.example"}]}]}
```
Result: `is_valid=false`, `manifest-not-substantive`, `conformance_level=minimal`.

```json
{"specVersion":"1.0","entries":[]}
```
Result: `is_valid=true`, `conformance_level=minimal`. A conformant document and a non-conformant one get the same level.

**Expected.** A document with errors meets no level. For example, `conformance_level` could be `Option<ConformanceLevel>`, or the enum could have a distinct variant for "not conformant", so that `Minimal` always means Level 1 is met.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::document_with_errors_is_not_met_where_they_report_minimal`.

---

## 6. Exceeding the nesting-depth limit is reported as a validity error (RD-08)

**Spec.** In [#nested-catalog-entries](#nested-catalog-entries): "Clients processing nested catalogs SHOULD impose a maximum nesting depth to prevent circular references. A depth limit of 4 is RECOMMENDED. Implementations MAY support deeper nesting but SHOULD document their limit." In [#organizing-catalogs](#organizing-catalogs): "…creating a hierarchy of any depth." [#nested-catalog-depth-and-circular-references](#nested-catalog-depth-and-circular-references) states the same limit as a SHOULD for clients.

**Current behaviour.** `ai-catalog-validate/src/lib.rs:350-359`: `if depth >= self.max_depth { self.error(Code::NestedDepth, …, "nested catalog depth exceeds recommended limit of {}") }`. This is an error, so `is_valid` is false and the level is `Minimal`. The check runs before the `data` check (`:362`), so a nested-catalog entry that uses `url` is also an error at the limit, even though `validate` does not fetch it.

**Reproduction.** Four inline nestings are accepted. Five inline nestings, or four inline nestings whose innermost catalog contains this entry:

```json
{"identifier":"urn:c","type":"application/ai-catalog+json","url":"https://e.example/c.json"}
```
Result: `is_valid=false`, `nested-depth-exceeded` at `catalog.entries[0].data.entries[0].data.entries[0].data.entries[0].data.entries[0]`.

**Expected.** Reaching the processing limit is reported as a warning that does not affect `is_valid` or the conformance level, because the document breaks no MUST. Nested content past the limit stays unvalidated, and the result says so.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::over_depth_inline_nesting_is_a_warning_for_us_and_an_error_for_them`.

---

## 7. The absolute-URI check does not follow RFC 3986 (RD-12)

**Spec.** In [#contributor-identity](#contributor-identity): "`contributor` MUST be an absolute URI [[RFC3986]] identifying the contributor…". In [#signature-object](#signature-object), `signer` is "An absolute URI [[RFC3986]] identifying the party claimed to have issued this endorsement."

**Current behaviour.** `ai-catalog-validate/src/lib.rs:456-461`:

```rust
Regex::new(r"^[A-Za-z][A-Za-z0-9+.\-]*:\S+$")
```

`contributor` (`manifest.rs:103`) and `signer` (`signature.rs:55`) are both checked with it. Any non-whitespace character is accepted after the scheme, and an empty hier-part is rejected.

**Reproduction.** Each value below was used as `contributor` in:

```json
{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"https://e.example/a","trustManifests":[{"contributor":"<value>","trustSchema":{"identifier":"s","version":"1"}}]}]}
```

| `contributor` | RFC 3986 | 0.4.0 |
|---|---|---|
| `urn:x:<bad>` | invalid (`<`, `>`) | accepted |
| `urn:x:a"b` | invalid (`"`) | accepted |
| `did:web:é.example` | invalid (non-ASCII) | accepted |
| `urn:x:%zz` | invalid (pct-encoded) | accepted |
| `http://[::1/x` | invalid (IP-literal) | accepted |
| `a:b#c#d` | invalid (`#` in fragment) | accepted |
| `a:{}\|^` + backtick | invalid | accepted |
| `foo:` | valid (`absolute-URI` with `path-empty`) | `manifest-contributor-not-uri` |

**Expected.** The check follows the RFC 3986 `absolute-URI` grammar, or uses a parser that implements it.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::contributor_outside_rfc3986_is_refused_by_us_only`.

---

## 8. Digest format: uppercase hex is accepted, and algorithms other than SHA-2 are rejected as invalid (RD-13)

**Spec.** In [#digest-format](#digest-format): "Digests in this specification use the format `algorithm:hex-value`, where `algorithm` is a hash algorithm identifier and `hex-value` is the lowercase hexadecimal encoding of the hash output." and "Consumers MUST reject digest values using algorithms shorter than SHA-256."

**Current behaviour.** `ai-catalog-trust/src/lib.rs:60-81`:
- `:72-73`: `hex_value.bytes().all(|byte| byte.is_ascii_hexdigit())` accepts `A`–`F`, and `:80` then lowercases the value.
- `:60` lowercases the algorithm name.
- `:66-69`: `md5`, `sha1` and `sha224` are `WeakDigestAlgorithm`, and every other name except `sha256`, `sha384` and `sha512` is `UnsupportedDigestAlgorithm`. `validate` reports both as the `digest-invalid` error (`ai-catalog-validate/src/manifest.rs:239-241`).

The MUST (reject algorithms shorter than SHA-256) is met.

**Reproduction.** Each value below was used as `digest` in `{"specVersion":"1.0","entries":[{"identifier":"urn:a","type":"application/json","url":"https://e.example/a","digest":"<value>"}]}`.

| `digest` | 0.4.0 |
|---|---|
| `sha256:9F86D081…0F00A08` (uppercase hex) | valid; `verify_digest` also returns `Ok(true)` |
| `SHA256:9f86d081…0f00a08` | valid |
| `sha1:a94a8fe5…` | `digest-invalid` (correct) |
| `sha3-256:9f86d081…` (256-bit output) | `digest-invalid` |
| `sha512-256:9f86d081…` (256-bit output) | `digest-invalid` |

**Expected.**
- Uppercase hex is reported, because the format specifies lowercase.
- Optionally, an algorithm whose output is not shorter than SHA-256 but which the implementation does not support could be reported apart from a malformed or weak digest, for example as a warning or a distinct code. The specification requires rejection only of algorithms shorter than SHA-256.

**Pinned by.** `crates/turul-ai-catalog/tests/sdk_interop.rs::uppercase_digest_hex_is_refused_by_us_only`.
