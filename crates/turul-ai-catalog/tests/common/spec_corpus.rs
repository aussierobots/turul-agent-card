//! The one list of spec examples and the rules that wrap each fragment for `parse`.

use std::fs;
use std::path::PathBuf;

pub const E: &str = r#""identifier":"urn:air:example.com:fixture:wrapper","type":"application/mcp-server-card+json","url":"https://example.com/wrapper.json""#;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rule {
    Doc,
    Entry,
    Host,
    TrustSchema,
    Attestation,
    Provenance,
    EntryMerge,
    /// Not parsed: a signed payload, not a stored object.
    Exempt,
}

use Rule::*;

/// Every fixture under `fixtures/spec/`, with the rule that feeds it to `parse`.
pub const SPEC_FIXTURES: &[(&str, Rule)] = &[
    ("examples/ai-catalog.json", Doc),
    // #top-level-structure
    ("blocks/01-top-level-structure.json", Doc),
    // #host-info
    ("blocks/02-host-info.json", Host),
    // #multi-version-entries
    ("blocks/03-multi-version-entries.json", Doc),
    // #catalog-projection
    ("blocks/04-catalog-projection.json", Entry),
    // #optional-members
    ("blocks/05-optional-members.json", Entry),
    // #trust-schema-object
    ("blocks/06-trust-schema-object.json", TrustSchema),
    // #attestation-object
    ("blocks/07-attestation-object.json", Attestation),
    // #provenance-link-object
    ("blocks/08-provenance-link-object.json", Provenance),
    // #signed-payload-and-jws-construction: its `signature` lacks `jws` by construction
    ("blocks/09-signed-payload-and-jws-construction.json", Exempt),
    // #nested-catalog-entries
    ("blocks/10-nested-catalog-entries.json", Doc),
    ("blocks/11-nested-catalog-entries.json", Entry),
    // #format-and-key-naming
    ("blocks/12-format-and-key-naming.json", Doc),
    // #policy-urls-extension: an `extensions` excerpt, merged into a wrapper entry
    ("blocks/13-policy-urls-extension.json", EntryMerge),
    // #unrecognized-members, "Examples"
    ("blocks/14-unrecognized-members-examples.json", Entry),
    ("blocks/15-unrecognized-members-examples.json", Entry),
    // #example-multi-artifact-catalog-with-nested-catalog
    (
        "blocks/16-example-multi-artifact-catalog-with-nested-catalog.json",
        Doc,
    ),
    // #example-hierarchical-catalog
    ("blocks/17-example-hierarchical-catalog.json", Doc),
    // #example-dual-protocol-agent-mcp-a2a
    ("blocks/18-example-dual-protocol-agent-mcp-a2a.json", Entry),
];

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spec")
}

pub fn bytes_of(name: &str) -> Vec<u8> {
    fs::read(fixture_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn rule_of(name: &str) -> Rule {
    SPEC_FIXTURES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("{name} unclassified"))
        .1
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn trust_prefix() -> String {
    format!(
        r#"{{"specVersion":"1.0","entries":[{{{E},"trustManifests":[{{"contributor":"did:web:example.com","#
    )
}

/// The bytes handed to `parse` for a fixture under its rule.
pub fn wrapped(name: &str) -> Vec<u8> {
    let frag = bytes_of(name);
    match rule_of(name) {
        Doc => frag,
        Entry => concat(&[br#"{"specVersion":"1.0","entries":["#, &frag, b"]}"]),
        Host => concat(&[
            br#"{"specVersion":"1.0","host":"#,
            &frag,
            br#","entries":[]}"#,
        ]),
        TrustSchema => {
            let p = format!(r#"{}"trustSchema":"#, trust_prefix());
            concat(&[p.as_bytes(), &frag, b"}]}]}"])
        }
        Attestation => {
            let p = format!(r#"{}"attestations":["#, trust_prefix());
            concat(&[p.as_bytes(), &frag, b"]}]}]}"])
        }
        Provenance => {
            let p = format!(r#"{}"provenance":["#, trust_prefix());
            concat(&[p.as_bytes(), &frag, b"]}]}]}"])
        }
        EntryMerge => {
            let at = frag
                .iter()
                .position(|b| *b == b'{')
                .expect("fragment has an object");
            let mut merged = frag[..at].to_vec();
            merged.extend_from_slice(b"{");
            merged.extend_from_slice(E.as_bytes());
            merged.push(b',');
            merged.extend_from_slice(&frag[at + 1..]);
            concat(&[br#"{"specVersion":"1.0","entries":["#, &merged, b"]}"])
        }
        Exempt => panic!("{name} is exempt"),
    }
}
