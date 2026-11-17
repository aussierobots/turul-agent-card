//! The one list of member names each object defines (#cddl-schema); `parse` leaves them out of
//! `unrecognized` and the writer never takes them from it.

pub(crate) const DOCUMENT: &[&str] = &[
    "specVersion",
    "host",
    "entries",
    "signature",
    "additionalSignatures",
    "extensions",
];
pub(crate) const HOST: &[&str] = &["displayName", "identifier", "documentationUrl", "logoUrl"];
pub(crate) const ENTRY: &[&str] = &[
    "identifier",
    "displayName",
    "type",
    "url",
    "data",
    "version",
    "description",
    "tags",
    "publisher",
    "digest",
    "trustManifests",
    "updatedAt",
    "extensions",
];
pub(crate) const PUBLISHER: &[&str] = &["identifier", "displayName", "identityType"];
pub(crate) const MANIFEST: &[&str] = &[
    "contributor",
    "subject",
    "signature",
    "additionalSignatures",
    "trustSchema",
    "attestations",
    "provenance",
    "extensions",
];
pub(crate) const SUBJECT: &[&str] = &["identifier", "type", "digest", "version"];
pub(crate) const SIGNATURE: &[&str] = &["signer", "profile", "issuedAt", "expiresAt", "jws"];
pub(crate) const TRUST_SCHEMA: &[&str] = &[
    "identifier",
    "version",
    "governanceUri",
    "verificationMethods",
];
pub(crate) const ATTESTATION: &[&str] = &["type", "uri", "digest", "size", "description"];
pub(crate) const PROVENANCE: &[&str] = &[
    "relation",
    "sourceId",
    "sourceDigest",
    "registryUri",
    "statementUri",
    "signatureRef",
];
