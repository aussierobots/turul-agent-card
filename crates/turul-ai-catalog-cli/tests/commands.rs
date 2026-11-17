//! CLI `validate`, `list` and `view` contract: exit codes, JSON bytes, text safety.
//! Every run is the real binary; expected JSON bytes are computed through the client's public
//! `load` and `to_json`.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use turul_ai_catalog_client::{FetchPolicy, Filter, Source, load};

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("binary runs")
}

fn run_file(command: &[&str], name: &str) -> Output {
    let path = fixture(name);
    let mut args: Vec<&str> = command.to_vec();
    args.push("--file");
    args.push(path.to_str().unwrap());
    run(&args)
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn expected_json(name: &str, filter: Option<&Filter>) -> Vec<u8> {
    let resolved = load(&Source::File(fixture(name)), &FetchPolicy::default()).expect("loads");
    let resolved = match filter {
        Some(f) => resolved.filter(f),
        None => resolved,
    };
    resolved.to_json().to_canonical_vec().expect("canonical")
}

// #top-level-structure: a catalog without error diagnostics is fine.
#[test]
fn validate_exits_0_for_a_catalog_without_errors() {
    let out = run_file(&["validate"], "ai-catalog.json");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
}

// #catalog-entry: an empty identifier is an error diagnostic.
#[test]
fn validate_exits_1_for_a_catalog_with_an_error_diagnostic() {
    let out = run_file(&["validate"], "diagnostics.json");
    assert_eq!(code(&out), 1);
    assert!(
        stdout(&out).contains("identifier-empty"),
        "{}",
        stdout(&out)
    );

    // An error inside an inline nested catalog in `data` counts too.
    let nested = run_file(&["validate"], "nested-error.json");
    assert_eq!(code(&nested), 1);
    assert!(
        stdout(&nested).contains("identifier-empty"),
        "{}",
        stdout(&nested)
    );
}

#[test]
fn list_exits_1_for_a_catalog_with_an_error_diagnostic() {
    let out = run_file(&["list"], "diagnostics.json");
    assert_eq!(code(&out), 1);
    assert!(!out.stdout.is_empty(), "output is written before exit 1");
}

#[test]
fn usage_error_exits_2() {
    let ai = fixture("ai-catalog.json");
    let ai = ai.to_str().unwrap();
    // No source flag.
    assert_eq!(code(&run(&["validate"])), 2);
    assert_eq!(code(&run(&["list"])), 2);
    // Two source flags.
    assert_eq!(
        code(&run(&[
            "validate",
            "--file",
            ai,
            "--url",
            "https://example.com/x.json"
        ])),
        2
    );
    assert_eq!(
        code(&run(&["list", "--file", ai, "--well-known", "example.com"])),
        2
    );
    // Unknown subcommand and unknown flag.
    assert_eq!(code(&run(&["frobnicate"])), 2);
    assert_eq!(code(&run(&["validate", "--file", ai, "--no-such-flag"])), 2);
}

#[test]
fn missing_file_exits_2() {
    let out = run(&["validate", "--file", "/nonexistent/turul/catalog.json"]);
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty(), "load failures go to stderr only");
    assert!(!out.stderr.is_empty());
}

#[test]
fn unparseable_file_exits_2() {
    let out = run_file(&["validate"], "unparseable.json");
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

// #safe-fetching: the CLI exposes no switch that loosens Safe Fetching.
#[test]
fn url_refused_by_safe_fetching_exits_2() {
    let out = run(&["validate", "--url", "http://127.0.0.1:9/"]);
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

// #signature-object: a Signature object missing a required member is a load failure.
#[test]
fn signature_missing_a_required_member_exits_2() {
    let out = run_file(&["validate"], "signature-missing-jws.json");
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
}

// #verification-result: a failure is never an endorsement, and it is never exit 1 by itself.
#[test]
fn validate_exits_0_when_only_signature_defects_are_present() {
    // A malformed `jws` header; `signer` `did:web:Example.com`.
    let out = run_file(&["validate"], "signature-defects.json");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let list = run_file(&["list"], "signature-defects.json");
    assert_eq!(code(&list), 0);
}

// #nested-catalogs: a skipped child is reported and is not an error of any loaded document.
#[test]
fn skipped_children_are_reported_and_do_not_change_the_exit_code() {
    let out = run_file(&["validate"], "skipped-offline.json");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.contains("https://127.0.0.1/x.json"), "{text}");
    assert!(text.contains("relative.json"), "{text}");
}

// A filter is a view: the exit code is decided on the unfiltered tree.
#[test]
fn filter_does_not_change_the_exit_code() {
    let out = run_file(
        &["list", "--artifact-type", "application/x-no-such-type"],
        "diagnostics.json",
    );
    assert_eq!(code(&out), 1);
    let clean = run_file(&["list", "--tag", "no-such-tag"], "ai-catalog.json");
    assert_eq!(code(&clean), 0);
}

// #embedded-content-safety and the canonical writer: a refused value means no output.
#[test]
fn json_output_refused_by_the_writer_exits_2() {
    let out = run_file(&["validate", "--format", "json"], "not-ijson-name.json");
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
    let list = run_file(&["list", "--format", "json"], "not-ijson-name.json");
    assert_eq!(code(&list), 2);
    assert!(list.stdout.is_empty());
}

#[test]
fn list_json_is_the_canonical_bytes_of_the_resolved_catalog() {
    let out = run_file(&["list", "--format", "json"], "ai-catalog.json");
    assert_eq!(code(&out), 0);
    assert_eq!(out.stdout, expected_json("ai-catalog.json", None));
    assert_ne!(out.stdout.last(), Some(&b'\n'), "no trailing newline");

    // The filtered tree is printed; one flag per `Filter` field.
    let mut filter = Filter::default();
    filter.tag = Some("dataset".to_string());
    let filtered = run_file(
        &["list", "--format", "json", "--tag", "dataset"],
        "ai-catalog.json",
    );
    assert_eq!(code(&filtered), 0);
    assert_eq!(
        filtered.stdout,
        expected_json("ai-catalog.json", Some(&filter))
    );
    assert_ne!(filtered.stdout, out.stdout);
}

#[test]
fn list_text_ends_each_entry_line_with_its_url() {
    // The fixture has no nested catalog, so its lines are exactly the root rows.
    let out = run_file(&["list"], "ai-catalog.json");
    assert_eq!(code(&out), 0);
    let tree = load(
        &Source::File(fixture("ai-catalog.json")),
        &FetchPolicy::default(),
    )
    .unwrap();
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), tree.rows.len());
    for (line, row) in lines.iter().zip(&tree.rows) {
        let url = match &tree.document.entries[row.index].source {
            turul_ai_catalog::model::ArtifactSource::Url(url) => url.as_str(),
            turul_ai_catalog::model::ArtifactSource::Data(_) => "-",
        };
        assert!(line.ends_with(&format!("  {url}")), "{line}");
    }
    assert!(text.contains("https://api.acme-corp.com/agents/acme-finance-agent.json"));

    // An entry with embedded `data` has no url.
    let nested = stdout(&run_file(&["list"], "nested-error.json"));
    let inline = nested
        .lines()
        .find(|l| l.contains("urn:air:example.com:catalog:inline"))
        .expect("the embedded entry is listed");
    assert!(inline.ends_with("  -"), "{inline}");
}

#[test]
fn validate_json_is_the_canonical_bytes_of_the_resolved_catalog() {
    let out = run_file(&["validate", "--format", "json"], "ai-catalog.json");
    assert_eq!(code(&out), 0);
    assert_eq!(out.stdout, expected_json("ai-catalog.json", None));
    assert_ne!(out.stdout.last(), Some(&b'\n'), "no trailing newline");
}

// #embedded-content-safety: every document-derived string in text output is control-free.
#[test]
fn control_characters_never_reach_text_output() {
    for command in [&["validate"][..], &["list"][..]] {
        let out = run_file(command, "control-characters.json");
        for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
            let text = String::from_utf8_lossy(bytes);
            let bad: Vec<char> = text
                .chars()
                .filter(|c| c.is_control() && *c != '\n')
                .collect();
            assert!(
                bad.is_empty(),
                "{command:?} {name} carries {bad:?}: {text:?}"
            );
        }
        assert!(stdout(&out).contains('\u{FFFD}'), "{command:?}");
    }
    // The pointer built from the `extensions` key holding ESC is printed with U+FFFD.
    let out = run_file(&["validate"], "control-characters.json");
    assert!(
        stdout(&out).contains("/extensions/\u{FFFD}[2J"),
        "{}",
        stdout(&out)
    );
}

// #embedded-content-safety: `view` needs stdin and stdout to be terminals.
#[test]
fn view_without_a_terminal_exits_2() {
    let path = fixture("ai-catalog.json");
    let out = Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"))
        .args(["view", "--file", path.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("binary runs");
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty(), "no escape sequence is written");
}
