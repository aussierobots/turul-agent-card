//! CLI producer commands `init`, `add`, `remove`: the binary runs in a fresh temporary directory
//! per test and results are read back with the format crate's `parse` and `to_canonical_vec`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::parse::parse;

const FILE: &str = "catalog.json";
const MINIMAL: &str = r#"{"entries":[],"specVersion":"1.0"}"#;

struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("turul-producer-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Dir(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.path(name), bytes).expect("write");
    }

    fn catalog(&self) -> Vec<u8> {
        fs::read(self.path(FILE)).expect("catalog.json exists")
    }

    fn catalog_text(&self) -> String {
        String::from_utf8(self.catalog()).expect("utf-8")
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"))
            .args(args)
            .current_dir(&self.0)
            .stdin(Stdio::null())
            .output()
            .expect("binary runs")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn init(dir: &Dir) {
    let out = dir.run(&["init", "--file", FILE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

fn add(dir: &Dir, id: &str, extra: &[&str]) -> Output {
    let mut args = vec![
        "add",
        "--file",
        FILE,
        "--identifier",
        id,
        "--type",
        "application/x-test",
        "--url",
        "https://example.com/a.json",
    ];
    args.extend_from_slice(extra);
    dir.run(&args)
}

fn member<'a>(json: &'a Json, name: &str) -> &'a Json {
    match json {
        Json::Object(m) => m.get(name).unwrap_or_else(|| panic!("no member {name}")),
        other => panic!("not an object: {other:?}"),
    }
}

fn entries(json: &Json) -> &[Json] {
    match member(json, "entries") {
        Json::Array(a) => a,
        other => panic!("entries is not an array: {other:?}"),
    }
}

fn without_entries(json: &Json) -> Json {
    match json {
        Json::Object(m) => {
            let mut out = turul_ai_catalog::model::Members::default();
            for (k, v) in m.iter().filter(|(k, _)| k.as_str() != "entries") {
                out.insert(k.clone(), v.clone());
            }
            Json::Object(out)
        }
        other => other.clone(),
    }
}

fn parsed_json(bytes: &[u8]) -> Json {
    read(bytes).expect("file is JSON")
}

fn hand_catalog(dir: &Dir, entries: &str) {
    dir.write(
        FILE,
        format!(r#"{{"specVersion":"1.0","entries":[{entries}]}}"#).as_bytes(),
    );
}

fn entry_json(id: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!(
            r#"{{"identifier":"{id}","type":"application/x-test","url":"https://example.com/{id}-{v}.json","version":"{v}"}}"#
        ),
        None => format!(
            r#"{{"identifier":"{id}","type":"application/x-test","url":"https://example.com/{id}.json"}}"#
        ),
    }
}

fn identifiers_and_versions(bytes: &[u8]) -> Vec<(String, Option<String>)> {
    let json = parsed_json(bytes);
    entries(&json)
        .iter()
        .map(|e| {
            let text = |n: &str| match e {
                Json::Object(m) => match m.get(n) {
                    Some(Json::String(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            };
            (text("identifier").unwrap(), text("version"))
        })
        .collect()
}

// #producer-behavior: "Producers MUST set `specVersion` to the version of this specification
// they implement".
#[test]
fn init_without_host_writes_the_minimal_catalog() {
    let dir = Dir::new("init-minimal");
    let out = dir.run(&["init", "--file", FILE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(dir.catalog_text(), MINIMAL);
    assert!(out.stdout.is_empty());
}

// #level-2-discoverable-catalog: "Includes a `host` object"; #host-info members.
#[test]
fn init_with_host_writes_a_discoverable_catalog() {
    let dir = Dir::new("init-host");
    let out = dir.run(&[
        "init",
        "--file",
        FILE,
        "--host-display-name",
        "Acme Services Inc.",
        "--host-identifier",
        "did:web:acme-corp.com",
        "--host-documentation-url",
        "https://docs.acme-corp.com/agents",
        "--host-logo-url",
        "data:image/png;base64,AAAA",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        dir.catalog_text(),
        concat!(
            r#"{"entries":[],"host":{"displayName":"Acme Services Inc.","#,
            r#""documentationUrl":"https://docs.acme-corp.com/agents","#,
            r#""identifier":"did:web:acme-corp.com","#,
            r#""logoUrl":"data:image/png;base64,AAAA"},"specVersion":"1.0"}"#
        )
    );
    let validated = dir.run(&["validate", "--file", FILE]);
    assert_eq!(code(&validated), 0, "{}", stderr(&validated));
    assert!(
        String::from_utf8_lossy(&validated.stdout).contains("Discoverable"),
        "{}",
        String::from_utf8_lossy(&validated.stdout)
    );
}

// #producer-behavior: "SHOULD NOT set `specVersion` to a version higher than they actually
// conform to"; add and remove keep the file's own value.
#[test]
fn add_and_remove_keep_the_spec_version_of_the_file() {
    for version in ["1.0", "1.1"] {
        let dir = Dir::new(&format!("keep-spec-{version}"));
        dir.write(
            FILE,
            format!(r#"{{"specVersion":"{version}","entries":[]}}"#).as_bytes(),
        );
        let out = add(&dir, "urn:air:example.com:x:a", &[]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let after_add = parsed_json(&dir.catalog());
        assert_eq!(
            member(&after_add, "specVersion"),
            &Json::String(version.into())
        );
        let out = dir.run(&[
            "remove",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:x:a",
        ]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let after_remove = parsed_json(&dir.catalog());
        assert_eq!(
            member(&after_remove, "specVersion"),
            &Json::String(version.into())
        );
    }
}

// #catalog-entry: every `add` flag maps to its member; absent flags give absent members.
#[test]
fn add_maps_each_flag_to_its_member() {
    let dir = Dir::new("add-maps");
    init(&dir);
    let out = add(&dir, "urn:air:example.com:x:min", &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        dir.catalog_text(),
        concat!(
            r#"{"entries":[{"identifier":"urn:air:example.com:x:min","#,
            r#""type":"application/x-test","url":"https://example.com/a.json"}],"#,
            r#""specVersion":"1.0"}"#
        )
    );

    let out = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:full",
        "--type",
        "application/x-other",
        "--url",
        "https://example.com/full.json",
        "--display-name",
        "Full Name",
        "--description",
        "A description.",
        "--version",
        "2.1",
        "--tag",
        "beta",
        "--tag",
        "alpha",
        "--tag",
        "beta",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let json = parsed_json(&dir.catalog());
    let full = &entries(&json)[1];
    let expected = read(
        concat!(
            r#"{"description":"A description.","displayName":"Full Name","#,
            r#""identifier":"urn:air:example.com:x:full","tags":["beta","alpha","beta"],"#,
            r#""type":"application/x-other","url":"https://example.com/full.json","version":"2.1"}"#
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(full, &expected);
}

// the file holds exactly the canonical bytes, no trailing newline.
#[test]
fn output_is_the_canonical_bytes_without_a_trailing_newline() {
    let dir = Dir::new("canonical");
    let check = |dir: &Dir| {
        let bytes = dir.catalog();
        assert_ne!(bytes.last(), Some(&b'\n'));
        let again = to_canonical_vec(&parse(&bytes).expect("parses")).expect("canonical");
        assert_eq!(bytes, again);
    };
    init(&dir);
    check(&dir);
    assert_eq!(
        code(&add(
            &dir,
            "urn:air:example.com:x:a",
            &["--description", "d\u{e9}"]
        )),
        0
    );
    check(&dir);
    assert_eq!(code(&add(&dir, "urn:air:example.com:x:b", &[])), 0);
    check(&dir);
    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:a",
    ]);
    assert_eq!(code(&out), 0);
    check(&dir);
}

// #multi-version-entries: identifier alone is unique when `version` is absent.
#[test]
fn add_with_a_duplicate_entry_key_is_refused_and_writes_nothing() {
    let dir = Dir::new("dup");
    init(&dir);
    assert_eq!(code(&add(&dir, "urn:air:example.com:x:a", &[])), 0);
    let before = dir.catalog();
    let out = add(&dir, "urn:air:example.com:x:a", &[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("entry-duplicate"), "{}", stderr(&out));
    assert_eq!(dir.catalog(), before);
    assert_eq!(dir.names(), [FILE]);
}

// #multi-version-entries: an unversioned entry beside a versioned one of the same identifier.
#[test]
fn add_of_an_unversioned_entry_beside_a_versioned_one_is_refused() {
    let dir = Dir::new("unversioned");
    init(&dir);
    assert_eq!(
        code(&add(&dir, "urn:air:example.com:x:a", &["--version", "1"])),
        0
    );
    let before = dir.catalog();
    let out = add(&dir, "urn:air:example.com:x:a", &[]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("version-missing-in-group"),
        "{}",
        stderr(&out)
    );
    assert_eq!(dir.catalog(), before);
}

// #multi-version-entries: (identifier, version) is the key when `version` is present.
#[test]
fn add_of_the_same_identifier_with_distinct_versions_is_written() {
    let dir = Dir::new("versions");
    init(&dir);
    assert_eq!(
        code(&add(&dir, "urn:air:example.com:x:a", &["--version", "1"])),
        0
    );
    let out = add(&dir, "urn:air:example.com:x:a", &["--version", "2"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        identifiers_and_versions(&dir.catalog()),
        [
            ("urn:air:example.com:x:a".to_string(), Some("1".to_string())),
            ("urn:air:example.com:x:a".to_string(), Some("2".to_string())),
        ]
    );
}

// #catalog-entry: an error diagnostic (empty identifier) refuses the write.
#[test]
fn add_with_an_error_diagnostic_is_refused_and_writes_nothing() {
    let dir = Dir::new("error-diag");
    init(&dir);
    let before = dir.catalog();
    let out = add(&dir, "", &[]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("identifier-empty"),
        "{}",
        stderr(&out)
    );
    assert_eq!(dir.catalog(), before);
    assert_eq!(dir.names(), [FILE]);
}

// #host-info: a warning never refuses the write.
#[test]
fn warnings_do_not_refuse_the_write() {
    let dir = Dir::new("warnings");
    let out = dir.run(&[
        "init",
        "--file",
        FILE,
        "--host-display-name",
        "Acme",
        "--host-logo-url",
        "https://example.com/logo.png",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("logo-url-not-data-uri"),
        "{}",
        stderr(&out)
    );
    assert!(dir.catalog_text().contains("https://example.com/logo.png"));
}

// #top-level-structure: `entries` is an ordered array; add appends.
#[test]
fn add_appends_entries_in_command_order() {
    let dir = Dir::new("order");
    init(&dir);
    for id in [
        "urn:air:example.com:x:c",
        "urn:air:example.com:x:a",
        "urn:air:example.com:x:b",
    ] {
        assert_eq!(code(&add(&dir, id, &[])), 0);
    }
    let ids: Vec<String> = identifiers_and_versions(&dir.catalog())
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        ids,
        [
            "urn:air:example.com:x:c",
            "urn:air:example.com:x:a",
            "urn:air:example.com:x:b"
        ]
    );
}

// #multi-version-entries: `remove` matches the entry key exactly; the rest keep their order.
#[test]
fn remove_deletes_only_the_entry_with_the_matching_key() {
    let x = "urn:air:example.com:x:x";
    let y = "urn:air:example.com:x:y";

    // An absent `--version` matches only the unversioned entry.
    let dir = Dir::new("remove-unversioned");
    hand_catalog(
        &dir,
        &[
            entry_json(x, None),
            entry_json(x, Some("1")),
            entry_json(x, Some("")),
        ]
        .join(","),
    );
    let out = dir.run(&["remove", "--file", FILE, "--identifier", x]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        identifiers_and_versions(&dir.catalog()),
        [
            (x.to_string(), Some("1".to_string())),
            (x.to_string(), Some(String::new()))
        ]
    );

    // `--version ""` matches only `version: ""`.
    let dir = Dir::new("remove-empty-version");
    hand_catalog(
        &dir,
        &[
            entry_json(y, None),
            entry_json(x, Some("")),
            entry_json(x, Some("1")),
            entry_json(y, None).replace(":y", ":z"),
        ]
        .join(","),
    );
    let out = dir.run(&["remove", "--file", FILE, "--identifier", x, "--version", ""]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        identifiers_and_versions(&dir.catalog()),
        [
            (y.to_string(), None),
            (x.to_string(), Some("1".to_string())),
            ("urn:air:example.com:x:z".to_string(), None),
        ]
    );
}

#[test]
fn remove_without_a_matching_key_exits_2() {
    let dir = Dir::new("remove-nomatch");
    init(&dir);
    assert_eq!(
        code(&add(&dir, "urn:air:example.com:x:a", &["--version", "1"])),
        0
    );
    let before = dir.catalog();
    for extra in [
        &["--identifier", "urn:air:example.com:x:other"][..],
        &["--identifier", "urn:air:example.com:x:a"][..],
        &["--identifier", "urn:air:example.com:x:a", "--version", "2"][..],
    ] {
        let mut args = vec!["remove", "--file", FILE];
        args.extend_from_slice(extra);
        let out = dir.run(&args);
        assert_eq!(code(&out), 2, "{extra:?}: {}", stderr(&out));
        assert_eq!(dir.catalog(), before);
    }
}

// #catalog-entry: `data` is any JSON value, opaque to the specification.
#[test]
fn add_data_file_is_embedded_as_its_json_value() {
    let dir = Dir::new("data-values");
    init(&dir);
    let values = [
        ("object", r#"{"b": [1, 2.5], "a": {"k": "v"}}"#),
        ("array", r#"[1, "two", null, true]"#),
        ("string", r#""just text""#),
        ("null", "null"),
    ];
    for (name, text) in values {
        let file = format!("{name}.data.json");
        dir.write(&file, text.as_bytes());
        let id = format!("urn:air:example.com:data:{name}");
        let out = dir.run(&[
            "add",
            "--file",
            FILE,
            "--identifier",
            &id,
            "--type",
            "application/x-test",
            "--data",
            &file,
        ]);
        assert_eq!(code(&out), 0, "{name}: {}", stderr(&out));
    }
    let json = parsed_json(&dir.catalog());
    for (i, (name, text)) in values.iter().enumerate() {
        assert_eq!(
            member(&entries(&json)[i], "data"),
            &read(text.as_bytes()).unwrap(),
            "{name}"
        );
        assert!(matches!(&entries(&json)[i], Json::Object(m) if m.get("url").is_none()));
    }
}

#[test]
fn add_data_file_that_is_not_json_exits_2() {
    let dir = Dir::new("data-not-json");
    init(&dir);
    let before = dir.catalog();
    for (name, text) in [("invalid", "{"), ("duplicate", r#"{"a":1,"a":2}"#)] {
        let file = format!("{name}.json");
        dir.write(&file, text.as_bytes());
        let out = dir.run(&[
            "add",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:data:x",
            "--type",
            "application/x-test",
            "--data",
            &file,
        ]);
        assert_eq!(code(&out), 2, "{name}: {}", stderr(&out));
        assert_eq!(dir.catalog(), before);
    }
    // A data file that cannot be read.
    let out = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:data:x",
        "--type",
        "application/x-test",
        "--data",
        "missing.json",
    ]);
    assert_eq!(code(&out), 2);
}

// RFC 8785 section 3.2.2.3: the canonical writer refuses a number that is not a finite double.
#[test]
fn add_non_i_json_data_exits_2() {
    let dir = Dir::new("data-not-ijson");
    init(&dir);
    let before = dir.catalog();
    dir.write("big.json", b"1e400");
    let out = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:data:big",
        "--type",
        "application/x-test",
        "--data",
        "big.json",
    ]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(dir.catalog(), before);
}

// RFC 8785 section 3.2.2.3: a number is written as the form of its nearest double, and the
// change of value is reported as a `number-inexact` warning (exit 0).
#[test]
fn add_inexact_data_number_is_written_in_its_rfc_8785_form() {
    let dir = Dir::new("data-inexact");
    init(&dir);
    dir.write("big.json", b"9007199254740993");
    let out = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:data:big",
        "--type",
        "application/x-test",
        "--data",
        "big.json",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stderr(&out).contains("number-inexact"), "{}", stderr(&out));
    assert!(
        dir.catalog_text().contains(r#""data":9007199254740992"#),
        "{}",
        dir.catalog_text()
    );
}

// #catalog-entry: "MUST contain exactly one of" `url` and `data`.
#[test]
fn add_requires_exactly_one_of_url_and_data() {
    let dir = Dir::new("one-of");
    init(&dir);
    let before = dir.catalog();
    dir.write("d.json", b"{}");
    let id = "urn:air:example.com:x:a";
    let neither = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        id,
        "--type",
        "application/x-test",
    ]);
    assert_eq!(code(&neither), 2);
    let both = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        id,
        "--type",
        "application/x-test",
        "--url",
        "https://example.com/a.json",
        "--data",
        "d.json",
    ]);
    assert_eq!(code(&both), 2);
    assert_eq!(dir.catalog(), before);
}

// #host-info: "It MUST contain: `displayName`".
#[test]
fn host_flags_without_display_name_are_a_usage_error() {
    for flag in [
        ["--host-identifier", "did:web:example.com"],
        ["--host-documentation-url", "https://example.com/docs"],
        ["--host-logo-url", "data:image/png;base64,AAAA"],
    ] {
        let dir = Dir::new("host-flag");
        let out = dir.run(&["init", "--file", FILE, flag[0], flag[1]]);
        assert_eq!(code(&out), 2, "{flag:?}");
        assert_eq!(dir.names(), Vec::<String>::new());
    }
}

// `init` never overwrites.
#[test]
fn init_refuses_an_existing_file() {
    let dir = Dir::new("init-exists");
    dir.write(FILE, b"existing bytes");
    let out = dir.run(&["init", "--file", FILE]);
    assert_eq!(code(&out), 2);
    assert_eq!(dir.catalog(), b"existing bytes");
}

// #catalog-snapshot-coverage: adding or removing "requires a new catalog signature".
#[test]
fn add_and_remove_refuse_a_signed_catalog() {
    let dir = Dir::new("signed");
    dir.write(
        FILE,
        concat!(
            r#"{"specVersion":"1.0","entries":[{"identifier":"urn:air:example.com:x:a","#,
            r#""type":"application/x-test","url":"https://example.com/a.json"}],"#,
            r#""signature":{"signer":"did:web:example.com","profile":"did-web-v1","#,
            r#""issuedAt":"2026-02-22T16:30:00Z","jws":"eyJhbGciOiJFUzI1NiJ9..c2ln"}}"#
        )
        .as_bytes(),
    );
    let before = dir.catalog();
    let out = add(&dir, "urn:air:example.com:x:b", &[]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert!(!out.stderr.is_empty());
    assert_eq!(dir.catalog(), before);
    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:a",
    ]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(dir.catalog(), before);
}

// #unrecognized-members, #consumer-behavior: preserved "when republishing".
#[test]
fn add_and_remove_preserve_members_they_do_not_set() {
    let dir = Dir::new("preserve");
    let original = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ai-catalog.json"),
    )
    .unwrap();
    let edits = [
        (
            r#""specVersion": "1.0","#,
            r#""specVersion": "1.0", "x-root": {"a": [1, 2]},"#,
        ),
        (r#""host": {"#, r#""host": {"x-host": true,"#),
        (
            r#""tags": ["finance", "trading"],"#,
            r#""tags": ["finance", "trading"], "x-entry": null, "extensions": {"com.example.ext": 1},"#,
        ),
    ];
    let mut text = original;
    for (from, to) in edits {
        assert_eq!(text.matches(from).count(), 1, "{from}");
        text = text.replace(from, to);
    }
    dir.write(FILE, text.as_bytes());
    let before = parsed_json(text.as_bytes());

    let out = add(&dir, "urn:air:example.com:x:new", &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let after_add = parsed_json(&dir.catalog());
    assert_eq!(without_entries(&after_add), without_entries(&before));
    assert_eq!(entries(&after_add).len(), 4);
    assert_eq!(&entries(&after_add)[..3], entries(&before));

    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:new",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let after_remove = parsed_json(&dir.catalog());
    assert_eq!(without_entries(&after_remove), without_entries(&before));
    assert_eq!(entries(&after_remove), entries(&before));

    // Removing another entry leaves the remaining ones, and the members around them, unchanged.
    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:acme-corp.com:data:market-2026q1",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let after_other = parsed_json(&dir.catalog());
    assert_eq!(without_entries(&after_other), without_entries(&before));
    assert_eq!(entries(&after_other)[0], entries(&before)[0]);
    assert_eq!(entries(&after_other)[1], entries(&before)[2]);
}

// #producer-behavior and ADR determinism: no command reads a clock.
#[test]
fn repeated_commands_give_identical_bytes() {
    let sequence = |dir: &Dir| {
        assert_eq!(
            code(&dir.run(&["init", "--file", FILE, "--host-display-name", "Acme"])),
            0
        );
        assert_eq!(
            code(&add(dir, "urn:air:example.com:x:a", &["--tag", "t"])),
            0
        );
        assert_eq!(
            code(&add(dir, "urn:air:example.com:x:b", &["--version", "1"])),
            0
        );
        let out = dir.run(&[
            "remove",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:x:a",
        ]);
        assert_eq!(code(&out), 0);
        dir.catalog()
    };
    let first = sequence(&Dir::new("repeat-1"));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = sequence(&Dir::new("repeat-2"));
    assert_eq!(first, second);
}

#[test]
fn usage_errors_exit_2() {
    let dir = Dir::new("usage");
    let cases: &[&[&str]] = &[
        &["init"],
        &["init", "--file", FILE, "--spec-version", "1.1"],
        &[
            "add",
            "--file",
            FILE,
            "--type",
            "application/x-test",
            "--url",
            "https://example.com/a",
        ],
        &[
            "add",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:x:a",
            "--url",
            "https://example.com/a",
        ],
        &["remove", "--file", FILE],
        &["remove", "--identifier", "urn:air:example.com:x:a"],
    ];
    for args in cases {
        let out = dir.run(args);
        assert_eq!(code(&out), 2, "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert_eq!(dir.names(), Vec::<String>::new());
}

#[test]
fn missing_target_file_exits_2() {
    let dir = Dir::new("missing-target");
    let out = add(&dir, "urn:air:example.com:x:a", &[]);
    assert_eq!(code(&out), 2);
    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:a",
    ]);
    assert_eq!(code(&out), 2);
    assert_eq!(dir.names(), Vec::<String>::new());
}

#[test]
fn producer_commands_write_nothing_to_stdout() {
    let dir = Dir::new("stdout-empty");
    let results = [
        dir.run(&[
            "init",
            "--file",
            FILE,
            "--host-display-name",
            "Acme",
            "--host-logo-url",
            "https://example.com/l.png",
        ]),
        add(&dir, "urn:air:example.com:x:a", &[]),
        add(&dir, "urn:air:example.com:x:a", &[]),
        add(&dir, "", &[]),
        dir.run(&[
            "remove",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:x:none",
        ]),
        dir.run(&[
            "remove",
            "--file",
            FILE,
            "--identifier",
            "urn:air:example.com:x:a",
        ]),
        dir.run(&["init", "--file", FILE]),
    ];
    let codes: Vec<i32> = results.iter().map(code).collect();
    assert_eq!(codes, [0, 0, 1, 1, 2, 0, 2]);
    for out in &results {
        assert!(
            out.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn no_temporary_file_remains() {
    let dir = Dir::new("no-temp");
    assert_eq!(code(&dir.run(&["init", "--file", FILE])), 0);
    assert_eq!(dir.names(), [FILE]);
    assert_eq!(code(&add(&dir, "urn:air:example.com:x:a", &[])), 0);
    assert_eq!(dir.names(), [FILE]);
    // A refusal (1).
    assert_eq!(code(&add(&dir, "urn:air:example.com:x:a", &[])), 1);
    assert_eq!(dir.names(), [FILE]);
    // A failure (2).
    let out = dir.run(&[
        "remove",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:x:none",
    ]);
    assert_eq!(code(&out), 2);
    dir.write("big.json", b"1e400");
    let out = dir.run(&[
        "add",
        "--file",
        FILE,
        "--identifier",
        "urn:air:example.com:data:big",
        "--type",
        "application/x-test",
        "--data",
        "big.json",
    ]);
    assert_eq!(code(&out), 2);
    assert_eq!(dir.names(), ["big.json", FILE]);
}

// #embedded-content-safety: every document-derived string on stderr passes through `DisplayText`.
// The ESC key is not reverse-DNS (#format-and-key-naming), so `extension-key-invalid` is an error
// and `add` is refused (exit 1) with the diagnostic on stderr.
#[test]
fn stderr_diagnostics_pass_through_display_text() {
    let dir = Dir::new("stderr-display");
    dir.write(
        FILE,
        br#"{"specVersion":"1.0","extensions":{"\u001b[2J":1},"entries":[]}"#,
    );
    let before = dir.catalog();
    let out = add(&dir, "urn:air:example.com:x:a", &[]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert_eq!(dir.catalog(), before);
    let text = stderr(&out);
    assert!(text.contains("/extensions/\u{FFFD}[2J"), "{text:?}");
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(
        text.chars().all(|c| !c.is_control() || c == '\n'),
        "{text:?}"
    );
}
