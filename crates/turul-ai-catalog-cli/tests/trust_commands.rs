//! CLI `verify`, `key` and `sign`: the real binary. `key` and `sign` (feature `ssm`) run
//! against a loopback stand-in for the Parameter Store endpoint. A DID document is only ever
//! fetched from `https://<domain>/.well-known/did.json`, so every end-to-end check of a
//! verified signature goes through the library with the resolved document supplied.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::json::read;
use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
use turul_ai_catalog_client::FetchPolicy;
#[cfg(feature = "ssm")]
use turul_ai_catalog_client::{Source, load};
use turul_ai_catalog_trust::{DidWebRoot, SignerParams, SigningKey, endorse_catalog};

#[cfg(feature = "ssm")]
#[path = "../../turul-ai-catalog-trust/tests/support/fake_ssm.rs"]
mod fake_ssm;

#[cfg(feature = "ssm")]
const SIGNER: &str = "did:web:example.com";

fn run_with(args: &[&str], env: &[(&str, String)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"))
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .output()
        .expect("binary runs")
}

fn run(args: &[&str]) -> Output {
    run_with(args, &[])
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("turul-trust-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn data_entry(identifier: &str) -> CatalogEntry {
    CatalogEntry::new(
        EntryIdentifier::new(identifier),
        ArtifactType::new("application/json"),
        ArtifactSource::Data(read(br#"{"name":"research"}"#).unwrap()),
    )
}

fn write_catalog(dir: &Path, name: &str, catalog: &AiCatalog) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, catalog.to_json().to_canonical_vec().unwrap()).unwrap();
    path
}

fn params(signer: &str) -> SignerParams {
    SignerParams {
        signer: DidWebRoot::parse(signer).unwrap(),
        key_fragment: "key-1".to_string(),
        issued_at: "2026-09-12T10:00:00Z".to_string(),
        expires_at: None,
    }
}

fn fixture_key() -> SigningKey {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turul-ai-catalog-trust/tests/fixtures/keys/rfc7515-a3.pk8");
    SigningKey::from_pkcs8_der(&std::fs::read(path).unwrap())
}

#[test]
fn verify_unsigned_entry_exits_0_and_prints_one_line_per_node_and_entry() {
    let dir = scratch("verify-unsigned");
    let catalog = AiCatalog::new(vec![data_entry("urn:air:example.com:fixture:a")]);
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let out = run(&["verify", "--file", file.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].ends_with("signature unsigned"), "{text}");
    assert_eq!(
        lines[1],
        "entry urn:air:example.com:fixture:a endorsement none integrity -"
    );
}

#[test]
fn verify_require_publisher_with_an_unsigned_entry_exits_1() {
    let dir = scratch("verify-require");
    let catalog = AiCatalog::new(vec![data_entry("urn:air:example.com:fixture:a")]);
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let out = run(&[
        "verify",
        "--file",
        file.to_str().unwrap(),
        "--require-publisher",
    ]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
}

// A nested catalog that was not loaded was not verified, so its entries cannot pass.
#[test]
fn verify_skipped_nested_catalog_exits_1() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skipped-offline.json");
    let out = run(&["verify", "--file", file.to_str().unwrap()]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stdout(&out).lines().any(|l| l.starts_with("skipped ")),
        "{}",
        stdout(&out)
    );
}

// A catalog embedded as `data` is not a loaded node, so its entries were not verified.
#[test]
fn verify_embedded_nested_catalog_exits_1() {
    let dir = scratch("verify-embedded");
    let inner = br#"{"specVersion":"1.0","entries":[{"identifier":"urn:air:example.com:fixture:b","type":"application/a2a-agent-card+json","url":"https://example.com/b.json"}]}"#;
    let outer = AiCatalog::new(vec![CatalogEntry::new(
        EntryIdentifier::new("urn:air:example.com:catalog:inner"),
        ArtifactType::AI_CATALOG,
        ArtifactSource::Data(read(inner).unwrap()),
    )]);
    let file = write_catalog(&dir, "catalog.json", &outer);
    let out = run(&["verify", "--file", file.to_str().unwrap()]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stdout(&out)
            .lines()
            .any(|l| l == "skipped embedded urn:air:example.com:catalog:inner"),
        "{}",
        stdout(&out)
    );
}

// An invalid catalog is not verified.
#[test]
fn verify_catalog_with_an_error_diagnostic_exits_1() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics.json");
    let out = run(&["verify", "--file", file.to_str().unwrap()]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
}

#[test]
fn verify_unreadable_source_exits_2() {
    let dir = scratch("verify-unreadable");
    let missing = dir.join("missing.json");
    let out = run(&["verify", "--file", missing.to_str().unwrap()]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty());
}

// A signature whose DID document cannot be retrieved is rejected, not skipped. `localhost` is
// refused by the default fetch policy before any connection, so no network is needed.
#[test]
fn verify_rejected_signature_exits_1() {
    let dir = scratch("verify-rejected");
    let mut entry = data_entry("urn:air:localhost:fixture:a");
    entry.digest = None;
    let catalog = endorse_catalog(
        AiCatalog::new(vec![entry]),
        None,
        &params("did:web:localhost"),
        &fixture_key(),
        &FetchPolicy::default(),
    )
    .unwrap();
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let out = run(&["verify", "--file", file.to_str().unwrap()]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("endorsement none"),
        "{}",
        stdout(&out)
    );
}

#[cfg(feature = "ssm")]
#[cfg(feature = "ssm")]
use std::time::{Duration, UNIX_EPOCH};

#[cfg(feature = "ssm")]
use turul_ai_catalog_client::Fetched;
#[cfg(feature = "ssm")]
use turul_ai_catalog_trust::{
    DidDocuments, Endorsement, Kid, SignatureOutcome, TrustPolicy, accept_document, select_key,
    verify_resolved,
};

#[cfg(feature = "ssm")]
use fake_ssm::FakeSsm;

#[cfg(feature = "ssm")]
const PARAMETER: &str = "/publisher/signing-key";
#[cfg(feature = "ssm")]
const FRAGMENT: &str = "key-1";

#[cfg(feature = "ssm")]
fn env(ssm: &FakeSsm) -> Vec<(&'static str, String)> {
    vec![
        ("AWS_ENDPOINT_URL_SSM", ssm.endpoint()),
        ("AWS_REGION", "us-east-1".to_string()),
        ("AWS_ACCESS_KEY_ID", "test".to_string()),
        ("AWS_SECRET_ACCESS_KEY", "test".to_string()),
        ("AWS_EC2_METADATA_DISABLED", "true".to_string()),
        ("AWS_CONFIG_FILE", "/dev/null".to_string()),
        ("AWS_SHARED_CREDENTIALS_FILE", "/dev/null".to_string()),
    ]
}

#[cfg(feature = "ssm")]
fn key_args<'a>(command: &'a str, fragment: &'a str) -> Vec<&'a str> {
    vec![
        "key",
        command,
        "--ssm-parameter",
        PARAMETER,
        "--did",
        SIGNER,
        "--fragment",
        fragment,
    ]
}

#[cfg(feature = "ssm")]
fn generate(ssm: &FakeSsm) -> Output {
    run_with(&key_args("generate", FRAGMENT), &env(ssm))
}

#[cfg(feature = "ssm")]
#[test]
fn key_generate_writes_the_parameter_and_prints_only_the_public_key() {
    let ssm = FakeSsm::start();
    let out = generate(&ssm);
    assert_eq!(code(&out), 0, "{}", stderr(&out));

    let pem = ssm.value(PARAMETER).expect("the parameter is written");
    assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    let body: Vec<_> = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let printed = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!printed.contains("PRIVATE"));
    assert!(body.iter().all(|l| !printed.contains(l)));
    assert!(!printed.contains("\"d\""));

    let requests = ssm.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "AmazonSSM.PutParameter");

    // The printed document is the one the verifier accepts, and the stored key prints it again.
    let root = DidWebRoot::parse(SIGNER).unwrap();
    let fetched = Fetched::new(
        200,
        Some("application/did+json".to_string()),
        out.stdout.clone(),
    );
    let document = accept_document(&fetched, &root).expect("accepted");
    let kid = Kid::parse(&format!("{SIGNER}#{FRAGMENT}"), &root).unwrap();
    select_key(&document, &kid).expect("the key is selected");

    let again = run_with(&key_args("did-document", FRAGMENT), &env(&ssm));
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(again.stdout, out.stdout);
}

#[cfg(feature = "ssm")]
#[test]
fn key_generate_with_an_existing_parameter_exits_2_and_writes_nothing() {
    let ssm = FakeSsm::start();
    ssm.seed(PARAMETER, "original");
    let out = generate(&ssm);
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
    assert_eq!(ssm.value(PARAMETER).as_deref(), Some("original"));
}

#[cfg(feature = "ssm")]
#[test]
fn key_generate_with_a_bad_fragment_exits_2_and_writes_nothing() {
    let ssm = FakeSsm::start();
    let out = run_with(&key_args("generate", "a b"), &env(&ssm));
    assert_eq!(code(&out), 2);
    assert!(ssm.requests().is_empty());
    assert_eq!(ssm.value(PARAMETER), None);
}

#[cfg(feature = "ssm")]
fn sign_args<'a>(file: &'a str, issued: &'a str, tail: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "sign",
        "--file",
        file,
        "--ssm-parameter",
        PARAMETER,
        "--did",
        SIGNER,
        "--fragment",
        FRAGMENT,
        "--issued-at",
        issued,
    ];
    args.extend_from_slice(tail);
    args
}

#[cfg(feature = "ssm")]
fn resolved(did_document: &[u8]) -> DidDocuments {
    let root = DidWebRoot::parse(SIGNER).unwrap();
    let fetched = Fetched::new(
        200,
        Some("application/did+json".to_string()),
        did_document.to_vec(),
    );
    let mut docs = DidDocuments::new();
    docs.insert(root.clone(), Ok(accept_document(&fetched, &root).unwrap()));
    docs
}

#[cfg(feature = "ssm")]
fn policy() -> TrustPolicy {
    let mut policy = TrustPolicy::default();
    policy.fetch.permit_loopback_http = true;
    policy
}

#[cfg(feature = "ssm")]
// 2026-09-12T10:00:00Z plus one second.
fn after_issue() -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_789_207_201)
}

#[cfg(feature = "ssm")]
#[test]
fn sign_output_verifies() {
    let dir = scratch("sign-verifies");
    let ssm = FakeSsm::start();
    let generated = generate(&ssm);
    assert_eq!(code(&generated), 0);
    let docs = resolved(&generated.stdout);

    let catalog = AiCatalog::new(vec![
        data_entry("urn:air:example.com:fixture:a"),
        data_entry("urn:air:other.example:fixture:b"),
    ]);
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let signed = run_with(
        &sign_args(file.to_str().unwrap(), "2026-09-12T10:00:00Z", &[]),
        &env(&ssm),
    );
    assert_eq!(code(&signed), 0, "{}", stderr(&signed));
    let out_file = dir.join("signed.json");
    std::fs::write(&out_file, &signed.stdout).unwrap();

    let tree = load(&Source::File(out_file.clone()), &FetchPolicy::default()).unwrap();
    let report = verify_resolved(&tree, &docs, &policy(), after_issue());
    let entries = &report.catalogs[0].entries;
    assert!(matches!(entries[0].integrity, Some(Ok(()))));
    assert_eq!(
        entries[0].manifests[0].endorsement(),
        Endorsement::Publisher
    );
    // An entry of another publisher is left as it was.
    assert!(entries[1].manifests.is_empty());
    assert!(entries[1].entry.digest.is_none());

    // Signing the signed file again is byte-identical, whatever the new issuedAt.
    let again = run_with(
        &sign_args(out_file.to_str().unwrap(), "2026-10-01T10:00:00Z", &[]),
        &env(&ssm),
    );
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(again.stdout, signed.stdout);
}

#[cfg(feature = "ssm")]
#[test]
fn sign_catalog_flag_signs_the_document_as_it_is() {
    let dir = scratch("sign-catalog");
    let ssm = FakeSsm::start();
    let generated = generate(&ssm);
    let docs = resolved(&generated.stdout);

    let catalog = AiCatalog::new(vec![data_entry("urn:air:example.com:fixture:a")]);
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let signed = run_with(
        &sign_args(
            file.to_str().unwrap(),
            "2026-09-12T10:00:00Z",
            &["--catalog"],
        ),
        &env(&ssm),
    );
    assert_eq!(code(&signed), 0, "{}", stderr(&signed));
    let out_file = dir.join("signed.json");
    std::fs::write(&out_file, &signed.stdout).unwrap();

    let tree = load(&Source::File(out_file), &FetchPolicy::default()).unwrap();
    assert!(tree.document.entries[0].trust_manifests.is_none());
    assert!(tree.document.entries[0].digest.is_none());
    let report = verify_resolved(&tree, &docs, &policy(), after_issue());
    assert!(matches!(
        report.catalogs[0].signature,
        SignatureOutcome::Accepted(_)
    ));
}

#[cfg(feature = "ssm")]
#[test]
fn sign_failures_exit_1_for_a_document_error_and_2_otherwise() {
    let dir = scratch("sign-failures");
    let ssm = FakeSsm::start();
    let generated = generate(&ssm);
    assert_eq!(code(&generated), 0);

    let diagnostics = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics.json");
    let out = run_with(
        &sign_args(diagnostics.to_str().unwrap(), "2026-09-12T10:00:00Z", &[]),
        &env(&ssm),
    );
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(out.stdout.is_empty());

    let catalog = AiCatalog::new(vec![data_entry("urn:air:example.com:fixture:a")]);
    let file = write_catalog(&dir, "catalog.json", &catalog);
    let empty = FakeSsm::start();
    let out = run_with(
        &sign_args(file.to_str().unwrap(), "2026-09-12T10:00:00Z", &[]),
        &env(&empty),
    );
    assert_eq!(code(&out), 2, "no key: {}", stderr(&out));
    assert!(out.stdout.is_empty());
}
