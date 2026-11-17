//! The one model to `Json` mapping. A member present in the model is written under its spec
//! name; `unrecognized` entries are written unchanged except a name the spec defines for that
//! object, which would otherwise shadow the typed member (#consumer-requirements).

use crate::json::{Json, Number};
use crate::model::*;
use crate::names;

/// Members under construction, seeded with the object's unrecognised members.
struct Out(Members);

impl Out {
    fn new(unrecognized: &Members, defined: &[&str]) -> Self {
        let mut m = Members::default();
        for (k, v) in unrecognized.iter() {
            if !defined.contains(&k.as_str()) {
                m.insert(k.clone(), v.clone());
            }
        }
        Self(m)
    }

    fn put(&mut self, name: &str, v: Json) {
        self.0.insert(name.to_string(), v);
    }

    fn text(&mut self, name: &str, v: &str) {
        self.put(name, Json::String(v.to_string()));
    }

    fn opt_text(&mut self, name: &str, v: &Option<String>) {
        if let Some(v) = v {
            self.text(name, v);
        }
    }

    fn opt<T>(&mut self, name: &str, v: &Option<T>, f: impl Fn(&T) -> Json) {
        if let Some(v) = v {
            self.put(name, f(v));
        }
    }

    fn opt_list<T>(&mut self, name: &str, v: &Option<Vec<T>>, f: impl Fn(&T) -> Json) {
        self.opt(name, v, |items| Json::Array(items.iter().map(&f).collect()));
    }

    fn done(self) -> Json {
        Json::Object(self.0)
    }
}

impl AiCatalog {
    /// The document as a `Json` value.
    pub fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::DOCUMENT);
        o.text("specVersion", &self.spec_version);
        o.opt("host", &self.host, HostInfo::to_json);
        o.put(
            "entries",
            Json::Array(self.entries.iter().map(CatalogEntry::to_json).collect()),
        );
        o.opt("signature", &self.signature, Signature::to_json);
        o.opt(
            "additionalSignatures",
            &self.additional_signatures,
            Json::clone,
        );
        o.opt("extensions", &self.extensions, |m| Json::Object(m.clone()));
        o.done()
    }
}

impl HostInfo {
    /// The Host Info as a `Json` value.
    pub fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::HOST);
        o.text("displayName", &self.display_name);
        if let Some(i) = &self.identifier {
            o.text("identifier", i.as_str());
        }
        o.opt_text("documentationUrl", &self.documentation_url);
        o.opt_text("logoUrl", &self.logo_url);
        o.done()
    }
}

impl CatalogEntry {
    /// The entry as a `Json` value.
    pub fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::ENTRY);
        o.text("identifier", self.identifier.as_str());
        o.opt_text("displayName", &self.display_name);
        o.text("type", self.artifact_type.as_str());
        match &self.source {
            ArtifactSource::Url(u) => o.text("url", u),
            ArtifactSource::Data(d) => o.put("data", d.clone()),
        }
        o.opt_text("version", &self.version);
        o.opt_text("description", &self.description);
        o.opt_list("tags", &self.tags, |s| Json::String(s.clone()));
        o.opt("publisher", &self.publisher, Publisher::to_json);
        o.opt_text("digest", &self.digest);
        o.opt_list(
            "trustManifests",
            &self.trust_manifests,
            TrustManifest::to_json,
        );
        o.opt_text("updatedAt", &self.updated_at);
        o.opt("extensions", &self.extensions, |m| Json::Object(m.clone()));
        o.done()
    }
}

impl Publisher {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::PUBLISHER);
        o.text("identifier", &self.identifier);
        o.text("displayName", &self.display_name);
        o.opt_text("identityType", &self.identity_type);
        o.done()
    }
}

impl TrustManifest {
    /// The manifest as a `Json` value.
    pub fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::MANIFEST);
        o.text("contributor", &self.contributor);
        o.opt("subject", &self.subject, Subject::to_json);
        o.opt("signature", &self.signature, Signature::to_json);
        o.opt(
            "additionalSignatures",
            &self.additional_signatures,
            Json::clone,
        );
        o.opt("trustSchema", &self.trust_schema, TrustSchema::to_json);
        o.opt_list("attestations", &self.attestations, Attestation::to_json);
        o.opt_list("provenance", &self.provenance, ProvenanceLink::to_json);
        o.opt("extensions", &self.extensions, |m| Json::Object(m.clone()));
        o.done()
    }
}

impl Subject {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::SUBJECT);
        o.text("identifier", self.identifier.as_str());
        o.text("type", self.artifact_type.as_str());
        o.text("digest", &self.digest);
        o.opt_text("version", &self.version);
        o.done()
    }
}

impl Signature {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::SIGNATURE);
        o.text("signer", &self.signer);
        o.text("profile", &self.profile);
        o.text("issuedAt", &self.issued_at);
        o.opt_text("expiresAt", &self.expires_at);
        o.text("jws", &self.jws);
        o.done()
    }
}

impl TrustSchema {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::TRUST_SCHEMA);
        o.text("identifier", &self.identifier);
        o.text("version", &self.version);
        o.opt_text("governanceUri", &self.governance_uri);
        o.opt_list("verificationMethods", &self.verification_methods, |s| {
            Json::String(s.clone())
        });
        o.done()
    }
}

impl Attestation {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::ATTESTATION);
        o.text("type", &self.attestation_type);
        o.text("uri", &self.uri);
        o.opt_text("digest", &self.digest);
        o.opt("size", &self.size, |n| {
            Json::Number(Number::new(n.to_string()))
        });
        o.opt_text("description", &self.description);
        o.done()
    }
}

impl ProvenanceLink {
    fn to_json(&self) -> Json {
        let mut o = Out::new(&self.unrecognized, names::PROVENANCE);
        o.text("relation", &self.relation);
        o.text("sourceId", &self.source_id);
        o.opt_text("sourceDigest", &self.source_digest);
        o.opt_text("registryUri", &self.registry_uri);
        o.opt_text("statementUri", &self.statement_uri);
        o.opt_text("signatureRef", &self.signature_ref);
        o.done()
    }
}
