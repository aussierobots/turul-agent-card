//! The `key`, `sign` and `verify` commands: arguments, key-store connection and printing. The
//! key, the DID document, the artifact digest, the endorsement and the verification are the
//! trust crate's `SigningKey`, `ParameterKeyStore`, `did_document`, `endorse_catalog`,
//! `sign_catalog` and `verify`.

use std::io::{self, Write};
use std::time::SystemTime;

use clap::Args;
#[cfg(feature = "ssm")]
use clap::Subcommand;
use turul_ai_catalog::identity::ArtifactType;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog_client::{Child, DisplayText, FetchPolicy, ResolvedCatalog, load};
#[cfg(feature = "ssm")]
use turul_ai_catalog_trust::{
    DidWebRoot, ParameterKeyStore, SignerParams, SigningKey, did_document, endorse_catalog,
    sign_catalog,
};
use turul_ai_catalog_trust::{Endorsement, SignatureOutcome, TrustPolicy, TrustReport, verify};

#[cfg(feature = "ssm")]
use crate::diagnostic_line;
use crate::{ERRORS, FAILED, OK, SourceArgs, fail, load_failed, skip_reason};

/// `key generate` and `key did-document`.
#[cfg(feature = "ssm")]
#[derive(Subcommand)]
pub enum KeyCommand {
    /// Generate a P-256 key into a new Parameter Store parameter and print its DID document.
    Generate {
        #[command(flatten)]
        key: KeyArgs,
    },
    /// Print the DID document of the key held in a Parameter Store parameter.
    DidDocument {
        #[command(flatten)]
        key: KeyArgs,
    },
}

/// Where the key is held and which DID and key fragment it signs as.
#[cfg(feature = "ssm")]
#[derive(Args)]
pub struct KeyArgs {
    /// The Parameter Store parameter holding the PKCS#8 PEM key.
    #[arg(long)]
    ssm_parameter: String,
    /// The root did:web DID of the signer.
    #[arg(long)]
    did: String,
    /// The fragment of the key id: the DID, `#`, then this.
    #[arg(long)]
    fragment: String,
}

/// `sign`.
#[cfg(feature = "ssm")]
#[derive(Args)]
pub struct SignArgs {
    #[command(flatten)]
    source: SourceArgs,
    #[command(flatten)]
    key: KeyArgs,
    /// The signature's issuedAt, RFC 3339.
    #[arg(long)]
    issued_at: String,
    /// The signature's expiresAt, RFC 3339.
    #[arg(long)]
    expires_at: Option<String>,
    /// Sign the whole catalog as it is instead of endorsing its entries.
    #[arg(long)]
    catalog: bool,
}

/// `verify`.
#[derive(Args)]
pub struct VerifyArgs {
    #[command(flatten)]
    source: SourceArgs,
    /// Exit 1 when any entry has no endorsement by its publisher.
    #[arg(long)]
    require_publisher: bool,
}

#[cfg(feature = "ssm")]
fn signer(did: &str) -> Result<DidWebRoot, String> {
    DidWebRoot::parse(did).ok_or_else(|| format!("{did} is not a root did:web DID"))
}

#[cfg(feature = "ssm")]
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(feature = "ssm")]
async fn parameter(name: &str) -> ParameterKeyStore {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    ParameterKeyStore::new(aws_sdk_ssm::Client::new(&config), name.to_string())
}

#[cfg(feature = "ssm")]
fn write_stdout(bytes: &[u8]) -> u8 {
    let mut out = io::stdout().lock();
    if out.write_all(bytes).and_then(|()| out.flush()).is_ok() {
        OK
    } else {
        FAILED
    }
}

/// Runs a `key` command and returns its exit code.
#[cfg(feature = "ssm")]
pub fn key(command: KeyCommand) -> u8 {
    let (args, generate) = match command {
        KeyCommand::Generate { key } => (key, true),
        KeyCommand::DidDocument { key } => (key, false),
    };
    let signer = match signer(&args.did) {
        Ok(signer) => signer,
        Err(message) => return fail(&message),
    };
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(message) => return fail(&message),
    };
    let key = if generate {
        SigningKey::generate()
    } else {
        match runtime.block_on(async { parameter(&args.ssm_parameter).await.load().await }) {
            Ok(key) => key,
            Err(e) => return fail(&e.to_string()),
        }
    };
    // The document is built before the key is stored, so a bad fragment writes nothing.
    let document = match key
        .public_jwk()
        .and_then(|jwk| did_document(&signer, &args.fragment, &jwk))
    {
        Ok(document) => document,
        Err(e) => return fail(&e.to_string()),
    };
    if generate
        && let Err(e) =
            runtime.block_on(async { parameter(&args.ssm_parameter).await.create(&key).await })
    {
        return fail(&e.to_string());
    }
    match document.to_canonical_vec() {
        Ok(bytes) => write_stdout(&bytes),
        Err(e) => fail(&turul_ai_catalog::error::chain(&e)),
    }
}

/// Runs `sign` and returns its exit code.
#[cfg(feature = "ssm")]
pub fn sign(args: SignArgs) -> u8 {
    let signer = match signer(&args.key.did) {
        Ok(signer) => signer,
        Err(message) => return fail(&message),
    };
    let (source, _) = match args.source.source() {
        Ok(s) => s,
        Err(message) => return fail(&message),
    };
    // Only the root is wanted: a file sends no request, a URL needs exactly its own.
    let mut policy = FetchPolicy::default();
    policy.max_requests = u32::from(!matches!(source, turul_ai_catalog_client::Source::File(_)));
    let tree = match load(&source, &policy) {
        Ok(tree) => tree,
        Err(e) => return load_failed(&e),
    };
    if tree.has_errors() {
        for d in tree.report.diagnostics() {
            eprintln!("{}", diagnostic_line(d));
        }
        return ERRORS;
    }
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(message) => return fail(&message),
    };
    let key =
        match runtime.block_on(async { parameter(&args.key.ssm_parameter).await.load().await }) {
            Ok(key) => key,
            Err(e) => return fail(&e.to_string()),
        };
    // Digest retrieval is blocking, so it runs outside the runtime.
    drop(runtime);
    let params = SignerParams {
        signer,
        key_fragment: args.key.fragment,
        issued_at: args.issued_at,
        expires_at: args.expires_at,
    };
    let base = tree.base.clone();
    let document = tree.document;
    let result = if args.catalog {
        sign_catalog(document, &params, &key).map_err(|e| turul_ai_catalog::error::chain(&e))
    } else {
        endorse_catalog(
            document,
            base.as_deref(),
            &params,
            &key,
            &FetchPolicy::default(),
        )
        .map_err(|e| turul_ai_catalog::error::chain(&e))
    };
    match result {
        Ok(catalog) => match catalog.to_json().to_canonical_vec() {
            Ok(bytes) => write_stdout(&bytes),
            Err(e) => fail(&turul_ai_catalog::error::chain(&e)),
        },
        Err(message) => fail(&message),
    }
}

/// Runs `verify` and returns its exit code.
pub fn verify_command(args: VerifyArgs) -> u8 {
    let (source, label) = match args.source.source() {
        Ok(s) => s,
        Err(message) => return fail(&message),
    };
    let tree = match load(&source, &FetchPolicy::default()) {
        Ok(tree) => tree,
        Err(e) => return load_failed(&e),
    };
    let report = verify(&tree, &TrustPolicy::default(), SystemTime::now());
    let mut stdout = io::stdout().lock();
    let rejected = match write_report(&report, &label, &mut stdout) {
        Ok(failed) => failed,
        Err(_) => return FAILED,
    };
    // A catalog that was not loaded, or that is invalid, was not verified.
    let unverified = match write_skipped(&tree, &mut stdout) {
        Ok(skipped) => skipped || tree.has_errors(),
        Err(_) => return FAILED,
    };
    if stdout.flush().is_err() {
        return FAILED;
    }
    if unverified
        || rejected.signature
        || rejected.integrity
        || (args.require_publisher && rejected.unendorsed)
    {
        ERRORS
    } else {
        OK
    }
}

/// Writes `skipped <reason> <url>` for every nested catalog of `tree` that was not loaded, and
/// `skipped embedded <identifier>` for every one embedded as `data`, and returns whether there
/// was one.
fn write_skipped(tree: &ResolvedCatalog, out: &mut dyn Write) -> io::Result<bool> {
    let mut any = false;
    // A catalog embedded as `data` is not loaded as a node, so its entries are never verified.
    for entry in &tree.document.entries {
        if entry.artifact_type == ArtifactType::AI_CATALOG
            && matches!(entry.source, ArtifactSource::Data(_))
        {
            writeln!(
                out,
                "skipped embedded {}",
                DisplayText::new(entry.identifier.as_str())
            )?;
            any = true;
        }
    }
    for n in &tree.children {
        match &n.child {
            Child::Catalog(c) => any |= write_skipped(c, out)?,
            Child::Skipped { url, reason } => {
                writeln!(
                    out,
                    "skipped {} {}",
                    DisplayText::new(&skip_reason(reason)),
                    DisplayText::new(url)
                )?;
                any = true;
            }
        }
    }
    Ok(any)
}

/// What the report holds that fails a verification.
#[derive(Default)]
struct Failures {
    signature: bool,
    integrity: bool,
    unendorsed: bool,
}

fn outcome_text(outcome: &SignatureOutcome<'_>) -> String {
    match outcome {
        SignatureOutcome::Unsigned => "unsigned".to_string(),
        SignatureOutcome::Accepted(_) => "accepted".to_string(),
        SignatureOutcome::Rejected(reason) => format!("rejected: {reason}"),
        _ => "unknown".to_string(),
    }
}

/// One line per catalog node (`catalog <source> signature <outcome>`) and per entry
/// (`entry <identifier> endorsement <publisher|contributor|none> integrity <ok|-|failure>`).
fn write_report(
    report: &TrustReport<'_>,
    root_label: &str,
    out: &mut dyn Write,
) -> io::Result<Failures> {
    let mut failures = Failures::default();
    for (index, node) in report.catalogs.iter().enumerate() {
        let label = if index == 0 {
            root_label
        } else {
            node.base.unwrap_or("(nested catalog)")
        };
        writeln!(
            out,
            "catalog {} signature {}",
            DisplayText::new(label),
            DisplayText::new(&outcome_text(&node.signature))
        )?;
        failures.signature |= matches!(node.signature, SignatureOutcome::Rejected(_));
        for entry in &node.entries {
            let level = entry.endorsement();
            let integrity = match &entry.integrity {
                None => "-".to_string(),
                Some(Ok(())) => "ok".to_string(),
                Some(Err(e)) => {
                    failures.integrity = true;
                    e.to_string()
                }
            };
            failures.signature |= entry
                .manifests
                .iter()
                .any(|m| matches!(m.signature, SignatureOutcome::Rejected(_)));
            failures.unendorsed |= level != Endorsement::Publisher;
            let level = match level {
                Endorsement::Publisher => "publisher",
                Endorsement::Contributor => "contributor",
                _ => "none",
            };
            writeln!(
                out,
                "entry {} endorsement {level} integrity {}",
                DisplayText::new(entry.entry.identifier.as_str()),
                DisplayText::new(&integrity)
            )?;
        }
    }
    Ok(failures)
}
