//! `ResolvedCatalog::to_json`: the one JSON output of a tree.

use std::collections::{BTreeMap, BTreeSet};

use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::model::{ArtifactSource, Members};
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, Severity};

use crate::tree::{Child, ResolvedCatalog, SkipReason};

/// The values each legend lists, ascending by code point.
#[derive(Default)]
struct Vocabulary {
    code: BTreeSet<String>,
    severity: BTreeSet<&'static str>,
    kind: BTreeSet<String>,
}

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn conformance(c: Conformance) -> &'static str {
    match c {
        Conformance::Met(ConformanceLevel::Minimal) => "minimal",
        Conformance::Met(ConformanceLevel::Discoverable) => "discoverable",
        Conformance::NotMet => "not-met",
        Conformance::Met(_) | Conformance::None | Conformance::Undetermined => "undetermined",
    }
}

fn skipped(r: &SkipReason) -> &'static str {
    match r {
        SkipReason::Budget => "budget",
        SkipReason::Depth => "depth",
        SkipReason::Cycle => "cycle",
        SkipReason::Unresolvable => "unresolvable",
        SkipReason::Fetch(_) => "fetch",
    }
}

fn int(n: usize) -> Json {
    read(n.to_string().as_bytes()).expect("a decimal integer is JSON")
}

fn text(s: &str) -> Json {
    Json::String(s.to_string())
}

fn opt(s: Option<&str>) -> Json {
    s.map_or(Json::Null, text)
}

fn object(members: Vec<(&str, Json)>) -> Json {
    let mut m = Members::default();
    for (k, v) in members {
        m.insert(k.to_string(), v);
    }
    Json::Object(m)
}

fn array<T>(items: impl IntoIterator<Item = T>, f: impl Fn(T) -> Json) -> Json {
    Json::Array(items.into_iter().map(f).collect())
}

fn legend<T: AsRef<str> + Ord>(set: &BTreeSet<T>) -> (Json, BTreeMap<&str, usize>) {
    let index = set
        .iter()
        .enumerate()
        .map(|(i, v)| (v.as_ref(), i))
        .collect();
    (array(set, |v| text(v.as_ref())), index)
}

impl ResolvedCatalog {
    /// The columnar JSON output; `to_canonical_vec` of it is the canonical bytes.
    pub fn to_json(&self) -> Json {
        let mut vocab = Vocabulary::default();
        self.collect(&mut vocab);
        let (code_legend, code) = legend(&vocab.code);
        let (severity_legend, severity) = legend(&vocab.severity);
        let (type_legend, kind) = legend(&vocab.kind);
        object(vec![
            (
                "legend",
                object(vec![
                    ("code", code_legend),
                    ("severity", severity_legend),
                    ("type", type_legend),
                ]),
            ),
            ("root", self.node(&code, &severity, &kind)),
        ])
    }

    fn collect(&self, v: &mut Vocabulary) {
        for w in &self.warnings {
            v.code.insert(w.code().to_string());
            v.severity.insert("warning");
        }
        for d in self.report.diagnostics() {
            v.code.insert(d.code.as_str().to_string());
            v.severity.insert(severity(d.severity));
        }
        for r in &self.rows {
            v.kind.insert(
                self.document.entries[r.index]
                    .artifact_type
                    .as_str()
                    .to_string(),
            );
        }
        for n in &self.children {
            if let Child::Catalog(c) = &n.child {
                c.collect(v);
            }
        }
    }

    fn node(
        &self,
        code: &BTreeMap<&str, usize>,
        severity_index: &BTreeMap<&str, usize>,
        kind: &BTreeMap<&str, usize>,
    ) -> Json {
        let entry = |i: usize| &self.document.entries[i];
        let entries = object(vec![
            ("index", array(&self.rows, |r| int(r.index))),
            (
                "identifier",
                array(&self.rows, |r| text(entry(r.index).identifier.as_str())),
            ),
            (
                "type",
                array(&self.rows, |r| {
                    int(kind[entry(r.index).artifact_type.as_str()])
                }),
            ),
            ("version", array(&self.rows, |r| opt(r.version.as_deref()))),
            ("name", array(&self.rows, |r| text(&r.name))),
            (
                "description",
                array(&self.rows, |r| opt(r.description.as_deref())),
            ),
            (
                "url",
                array(&self.rows, |r| match &entry(r.index).source {
                    ArtifactSource::Url(u) => text(u),
                    ArtifactSource::Data(_) => Json::Null,
                }),
            ),
        ]);

        // Warnings first (path ""), then the report in order.
        let mut rows: Vec<(&str, &str, String)> = self
            .warnings
            .iter()
            .map(|w| ("warning", w.code(), String::new()))
            .collect();
        rows.extend(
            self.report
                .diagnostics()
                .iter()
                .map(|d| (severity(d.severity), d.code.as_str(), d.path.to_string())),
        );
        let diagnostics = object(vec![
            ("severity", array(&rows, |r| int(severity_index[r.0]))),
            ("code", array(&rows, |r| int(code[r.1]))),
            ("path", array(&rows, |r| text(&r.2))),
        ]);

        let children = array(&self.children, |n| match &n.child {
            Child::Catalog(c) => object(vec![
                ("entry", int(n.entry)),
                ("catalog", c.node(code, severity_index, kind)),
            ]),
            Child::Skipped { url, reason } => object(vec![
                ("entry", int(n.entry)),
                ("url", text(url)),
                ("skipped", text(skipped(reason))),
            ]),
        });

        object(vec![
            ("source", opt(self.base.as_deref())),
            ("conformance", text(conformance(self.report.conformance()))),
            ("entries", entries),
            ("diagnostics", diagnostics),
            ("children", children),
        ])
    }
}
