# Interop fixtures

Authored in this repository (Apache-2.0). Each file is the smallest document that exercises its id.
Digests are the `sha256` of `test`; signatures reuse the structurally valid `did-web-v1` JWS values of `upstream/trust_clean.json`.

| File | Exercises |
|---|---|
| `unknown-members-every-kind.json` | #unrecognized-members, an unknown `zz` member on all 11 object kinds (12 members: Signature on the catalog and on the manifest) |
| `empty-arrays.json` | RD-03: empty `tags`, `trustManifests`, `attestations`, `provenance`, `verificationMethods` |
| `data-null.json` | RD-02, AMB-09: `"data": null` |
| `url-empty.json` | RD-02: `"url": ""` |
| `nested-signed-manifest.json` | #nested-catalog-entries: inline nested catalog holding an entry with a digest and a signed Trust Manifest |
| `missing-entries.json` | RD-01: `{"specVersion":"1.0"}` |
| `duplicate-member.json` | RD-04: duplicate member name inside `data` |
| `spec-version-plus.json` | RD-14: `"specVersion": "+1.0"` |
| `lone-surrogate.json` | AMB-82: `"\ud800"` in an `identifier` |
| `digest-uppercase.json` | RD-13: entry `digest` with uppercase hex |
| `contributor-not-rfc3986.json` | RD-12: `contributor` `urn:acme|team` |
