#![doc = include_str!("../README.md")]

mod store;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier, EntryKey};
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::validate::{Diagnostic, Severity};
use turul_ai_catalog_client::{
    Change, Child, Content, DisplayText, EntryInput, FetchPolicy, Filter, HostInput, LoadError,
    ProduceError, ResolvedCatalog, Source, WellKnownHost, load, produce,
};
use turul_ai_catalog_tui::RunError;

const OK: u8 = 0;
const ERRORS: u8 = 1;
const FAILED: u8 = 2;

#[derive(Parser)]
#[command(
    name = "turul-ai-catalog",
    version,
    about = "Work with AI Catalog documents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a catalog and every catalog nested in it.
    Validate {
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// List the entries of a catalog tree, optionally filtered.
    List {
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
        #[command(flatten)]
        filter: FilterArgs,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Browse a catalog tree in the terminal.
    View {
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Create an empty catalog file.
    Init {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        host_display_name: Option<String>,
        #[arg(long, requires = "host_display_name")]
        host_identifier: Option<String>,
        #[arg(long, requires = "host_display_name")]
        host_documentation_url: Option<String>,
        #[arg(long, requires = "host_display_name")]
        host_logo_url: Option<String>,
    },
    /// Append an entry to a catalog file.
    Add {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        identifier: String,
        #[arg(long = "type")]
        artifact_type: String,
        #[command(flatten)]
        content: ContentArgs,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Remove the entry with this identifier and version from a catalog file.
    Remove {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        identifier: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// List, remove, import or purge the entries of a catalog held in a store.
    #[command(subcommand)]
    Store(store::StoreCommand),
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct SourceArgs {
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    well_known: Option<String>,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct ContentArgs {
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    data: Option<PathBuf>,
}

#[derive(Args)]
struct FilterArgs {
    #[arg(long)]
    identifier: Option<String>,
    #[arg(long)]
    version: Option<String>,
    #[arg(long)]
    artifact_type: Option<String>,
    #[arg(long)]
    tag: Option<String>,
}

impl FilterArgs {
    fn filter(self) -> Filter {
        let mut filter = Filter::default();
        filter.identifier = self.identifier.map(EntryIdentifier::new);
        filter.version = self.version;
        filter.artifact_type = self.artifact_type.map(ArtifactType::new);
        filter.tag = self.tag;
        filter
    }
}

impl SourceArgs {
    fn source(self) -> Result<(Source, String), String> {
        match (self.file, self.url, self.well_known) {
            (Some(file), _, _) => {
                let label = file.display().to_string();
                Ok((Source::File(file), label))
            }
            (_, Some(url), _) => Ok((Source::Url(url.clone()), url)),
            (_, _, Some(host)) => {
                let source = WellKnownHost::new(&host)
                    .map(Source::WellKnown)
                    .map_err(|e| turul_ai_catalog::error::chain(&e))?;
                Ok((source, host))
            }
            _ => Err("exactly one source flag is required".to_string()),
        }
    }
}

/// Parses the command line, runs the command and maps its result to the exit code.
fn main() -> ExitCode {
    ExitCode::from(run(Cli::parse().command))
}

/// Runs one command and returns its exit code.
fn run(command: Command) -> u8 {
    match command {
        Command::Store(command) => store::run(command),
        Command::Validate { format, source } => {
            read_only(source, format, None, |tree, label, out| {
                validate_text(tree, label, out)
            })
        }
        Command::List {
            format,
            filter,
            source,
        } => read_only(source, format, Some(filter.filter()), |tree, _, out| {
            list_text(tree, out)
        }),
        Command::View { source } => view(source),
        Command::Init {
            file,
            host_display_name,
            host_identifier,
            host_documentation_url,
            host_logo_url,
        } => {
            let host = host_display_name.map(|name| {
                let mut host = HostInput::new(name);
                host.identifier = host_identifier;
                host.documentation_url = host_documentation_url;
                host.logo_url = host_logo_url;
                host
            });
            produced(&file, &Change::Init { host })
        }
        Command::Add {
            file,
            identifier,
            artifact_type,
            content,
            display_name,
            description,
            version,
            tags,
        } => {
            let content = match (content.url, content.data) {
                (Some(url), _) => Content::Url(url),
                (_, Some(data)) => Content::DataFile(data),
                _ => return fail("exactly one of --url and --data is required"),
            };
            let mut entry = EntryInput::new(identifier, artifact_type, content);
            entry.display_name = display_name;
            entry.description = description;
            entry.version = version;
            entry.tags = tags;
            produced(&file, &Change::Add(entry))
        }
        Command::Remove {
            file,
            identifier,
            version,
        } => produced(
            &file,
            &Change::Remove(EntryKey::new(EntryIdentifier::new(identifier), version)),
        ),
    }
}

/// Writes `message` to stderr and returns the failure exit code.
fn fail(message: &str) -> u8 {
    eprintln!("{}", DisplayText::new(message));
    FAILED
}

/// `validate` and `list`: load, warn on stderr, write the output, then the exit rule.
fn read_only(
    source: SourceArgs,
    format: Format,
    filter: Option<Filter>,
    text: impl Fn(&ResolvedCatalog, &str, &mut dyn Write) -> io::Result<()>,
) -> u8 {
    let (source, label) = match source.source() {
        Ok(s) => s,
        Err(message) => return fail(&message),
    };
    let tree = match load(&source, &FetchPolicy::default()) {
        Ok(tree) => tree,
        Err(e) => return load_failed(&e),
    };
    let _ = write_warnings(&tree, &mut io::stderr());
    let shown = filter
        .as_ref()
        .map_or_else(|| tree.clone(), |f| tree.filter(f));
    let mut stdout = io::stdout().lock();
    let written = match format {
        Format::Text => text(&shown, &label, &mut stdout),
        Format::Json => match shown.to_json().to_canonical_vec() {
            Ok(bytes) => stdout.write_all(&bytes),
            Err(e) => return fail(&turul_ai_catalog::error::chain(&e)),
        },
    }
    .and_then(|()| stdout.flush());
    if written.is_err() {
        return FAILED;
    }
    exit_code(&tree)
}

/// Loads the tree and hands it to the terminal browser; the exit code follows the loaded tree.
fn view(source: SourceArgs) -> u8 {
    let (source, _) = match source.source() {
        Ok(s) => s,
        Err(message) => return fail(&message),
    };
    let tree = match load(&source, &FetchPolicy::default()) {
        Ok(tree) => tree,
        Err(e) => return load_failed(&e),
    };
    let code = exit_code(&tree);
    match turul_ai_catalog_tui::run(tree) {
        Ok(()) => code,
        Err(RunError::NotATerminal) => fail(&RunError::NotATerminal.to_string()),
        Err(e) => fail(&turul_ai_catalog::error::chain(&e)),
    }
}

/// Reports a load failure on stderr and returns the failure exit code.
fn load_failed(e: &LoadError) -> u8 {
    fail(&turul_ai_catalog::error::chain(&e))
}

/// One rule for every command: 1 when any loaded catalog has an error diagnostic.
fn exit_code(tree: &ResolvedCatalog) -> u8 {
    if tree.has_errors() { ERRORS } else { OK }
}

/// Applies `change` to `file`, prints the resulting diagnostics to stderr and returns the exit code.
fn produced(file: &std::path::Path, change: &Change) -> u8 {
    match produce(file, change) {
        Ok(report) => {
            for d in report.diagnostics() {
                eprintln!("{}", diagnostic_line(d));
            }
            OK
        }
        Err(ProduceError::Invalid(report)) => {
            for d in report.diagnostics() {
                eprintln!("{}", diagnostic_line(d));
            }
            ERRORS
        }
        Err(e) => fail(&turul_ai_catalog::error::chain(&e)),
    }
}

/// `warning <code>: <Display>` per load warning, root first then tree order.
fn write_warnings(tree: &ResolvedCatalog, out: &mut dyn Write) -> io::Result<()> {
    for w in &tree.warnings {
        writeln!(
            out,
            "warning {}: {}",
            DisplayText::new(w.code()),
            DisplayText::new(&w.to_string())
        )?;
    }
    for n in &tree.children {
        if let Child::Catalog(c) = &n.child {
            write_warnings(c, out)?;
        }
    }
    Ok(())
}

/// Formats one diagnostic as `<severity> <code> <pointer>`.
fn diagnostic_line(d: &Diagnostic) -> String {
    let severity = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    format!(
        "{severity} {} {}",
        DisplayText::new(d.code.as_str()),
        DisplayText::new(&d.path.to_string())
    )
}

/// Writes the conformance, diagnostics and skipped children of every catalog node.
fn validate_text(tree: &ResolvedCatalog, label: &str, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "{}: {:?}",
        DisplayText::new(label),
        tree.report.conformance()
    )?;
    for d in tree.report.diagnostics() {
        writeln!(out, "  {}", diagnostic_line(d))?;
    }
    for n in &tree.children {
        match &n.child {
            Child::Catalog(c) => {
                let child_label = c.base.as_deref().unwrap_or("(nested catalog)");
                validate_text(c, child_label, out)?;
            }
            Child::Skipped { url, reason } => {
                let reason = match reason {
                    turul_ai_catalog_client::SkipReason::Budget => "budget".to_string(),
                    turul_ai_catalog_client::SkipReason::Depth => "depth".to_string(),
                    turul_ai_catalog_client::SkipReason::Cycle => "cycle".to_string(),
                    turul_ai_catalog_client::SkipReason::Unresolvable => "unresolvable".to_string(),
                    turul_ai_catalog_client::SkipReason::Fetch(e) => format!("fetch: {e}"),
                    _ => "skipped".to_string(),
                };
                writeln!(
                    out,
                    "  skipped {} {}",
                    DisplayText::new(&reason),
                    DisplayText::new(url)
                )?;
            }
        }
    }
    Ok(())
}

/// Writes one line per entry row of every catalog node: name, identifier, type, version and the
/// entry's `url` (`-` for embedded `data`).
fn list_text(tree: &ResolvedCatalog, out: &mut dyn Write) -> io::Result<()> {
    for row in &tree.rows {
        let entry = &tree.document.entries[row.index];
        let url = match &entry.source {
            ArtifactSource::Url(url) => url.as_str(),
            ArtifactSource::Data(_) => "-",
        };
        writeln!(
            out,
            "{}  {}  {}  {}  {}",
            DisplayText::new(&row.name),
            DisplayText::new(entry.identifier.as_str()),
            DisplayText::new(entry.artifact_type.as_str()),
            DisplayText::new(row.version.as_deref().unwrap_or("-")),
            DisplayText::new(url),
        )?;
    }
    for n in &tree.children {
        if let Child::Catalog(c) = &n.child {
            list_text(c, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use turul_ai_catalog_client::LoadWarning;

    // A load warning is one stderr line and never changes the exit code.
    #[test]
    fn load_warnings_are_one_stderr_line_each_and_keep_the_exit_code() {
        let path: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "tests",
            "fixtures",
            "ai-catalog.json",
        ]
        .iter()
        .collect();
        let mut tree = load(&Source::File(path), &FetchPolicy::default()).expect("loads");
        let warning = LoadWarning::RequestBudget {
            limit: 4,
            skipped: 3,
        };
        tree.warnings.push(warning);
        let mut out = Vec::new();
        write_warnings(&tree, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("warning request-budget: {warning}\n")
        );
        assert_eq!(exit_code(&tree), OK);
    }
}
