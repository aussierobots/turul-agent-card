//! The `store` command: arguments, backend connection and printing. Listing, removal, import and
//! purge are `turul_ai_catalog_storage::list_registrations`, `remove_registration`, `import` and
//! `CatalogStore::purge_catalog`.

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use clap::{Args, Subcommand};
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog_client::{DisplayText, FetchPolicy, Source, load};
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, ImportError, ImportMode, ItemCounts, Provenance, WriterId, import,
    list_registrations, remove_registration,
};

use crate::{ERRORS, FAILED, OK, fail, load_failed};

/// The variable that names the catalog id a deployed function serves, and the fallback of
/// `--catalog-id`.
const LIVE_CATALOG_ID: &str = "TURUL_AI_CATALOG_ID";

#[derive(Subcommand)]
pub enum StoreCommand {
    /// List the entries of a catalog in a store with their writer and expiry state.
    List {
        #[command(flatten)]
        target: Target,
    },
    /// Remove the entry with this identifier and version from a store.
    Remove {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        identifier: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// Store the entries of a catalog file as curated entries of a writer.
    Import {
        #[command(flatten)]
        target: Target,
        /// The catalog file to import.
        #[arg(long)]
        file: PathBuf,
        /// The permanent writer id the entries are curated by.
        #[arg(long)]
        writer: String,
        /// Make the catalog id hold exactly the file: overwrite any entry, delete the others,
        /// refuse while a lease is live. A file without a host keeps the stored host.
        #[arg(long)]
        replace: bool,
    },
    /// Remove every item of a catalog id; without --yes, only count them. --yes requires
    /// TURUL_AI_CATALOG_ID (the served id, which cannot be purged).
    Purge {
        #[command(flatten)]
        target: Target,
        /// Remove the items instead of previewing.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct Target {
    #[command(flatten)]
    backend: Backend,
    /// The catalog id within the store.
    #[arg(long, env = "TURUL_AI_CATALOG_ID")]
    catalog_id: String,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Backend {
    /// A SQLite database file.
    #[arg(long, env = "TURUL_AI_CATALOG_SQLITE")]
    sqlite: Option<String>,
    /// A PostgreSQL connection URL.
    #[cfg(feature = "postgres")]
    #[arg(long, env = "TURUL_AI_CATALOG_POSTGRES_URL")]
    postgres: Option<String>,
    /// A DynamoDB table name.
    #[cfg(feature = "dynamodb")]
    #[arg(long = "table", env = "TURUL_AI_CATALOG_TABLE")]
    table: Option<String>,
}

impl Backend {
    async fn connect(self) -> Result<Box<dyn CatalogStore>, String> {
        let failed = |e: turul_ai_catalog_storage::StoreError| e.to_string();
        if let Some(path) = self.sqlite {
            return turul_ai_catalog_storage::SqliteCatalogStore::connect(&path)
                .await
                .map(|s| Box::new(s) as Box<dyn CatalogStore>)
                .map_err(failed);
        }
        #[cfg(feature = "postgres")]
        if let Some(url) = self.postgres {
            return turul_ai_catalog_storage::PostgresCatalogStore::connect(&url)
                .await
                .map(|s| Box::new(s) as Box<dyn CatalogStore>)
                .map_err(failed);
        }
        #[cfg(feature = "dynamodb")]
        if let Some(table) = self.table {
            let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let client = aws_sdk_dynamodb::Client::new(&config);
            return Ok(Box::new(
                turul_ai_catalog_storage::DynamoDbCatalogStore::new(client, table),
            ));
        }
        Err("exactly one backend is required".to_string())
    }
}

impl Target {
    async fn open(self) -> Result<(Box<dyn CatalogStore>, CatalogId), String> {
        let catalog = CatalogId::new(self.catalog_id)
            .map_err(|e| format!("the catalog id is not valid: {e}"))?;
        Ok((self.backend.connect().await?, catalog))
    }
}

/// Runs a `store` command and returns its exit code.
pub fn run(command: StoreCommand) -> u8 {
    // The file is read before the runtime exists: `load` is blocking.
    let command = match command {
        StoreCommand::Import {
            target,
            file,
            writer,
            replace,
        } => {
            let mut policy = FetchPolicy::default();
            // No request is sent: nested catalogs are imported as entries, never followed.
            policy.max_requests = 0;
            match load(&Source::File(file), &policy) {
                Ok(tree) => Loaded::Import {
                    target,
                    document: Box::new(tree.document),
                    writer,
                    replace,
                },
                Err(e) => return load_failed(&e),
            }
        }
        other => Loaded::Other(other),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return fail(&e.to_string()),
    };
    runtime.block_on(execute(command))
}

/// A `store` command with its catalog file already read.
enum Loaded {
    Import {
        target: Target,
        document: Box<turul_ai_catalog::model::AiCatalog>,
        writer: String,
        replace: bool,
    },
    Other(StoreCommand),
}

async fn import_document(
    target: Target,
    document: turul_ai_catalog::model::AiCatalog,
    writer: String,
    replace: bool,
) -> u8 {
    let writer = match WriterId::new(writer) {
        Ok(writer) => writer,
        Err(e) => return fail(&format!("the writer id is not valid: {e}")),
    };
    let (store, catalog) = match target.open().await {
        Ok(opened) => opened,
        Err(message) => return fail(&message),
    };
    let mode = if replace {
        ImportMode::Replace {
            now: SystemTime::now(),
        }
    } else {
        ImportMode::Merge
    };
    match import(&*store, &catalog, &document, &writer, mode).await {
        Ok(_) => {
            println!("imported {} entries", document.entries.len());
            OK
        }
        Err(ImportError::Invalid { errors }) => {
            for error in &errors {
                eprintln!("{}", DisplayText::new(error));
            }
            ERRORS
        }
        Err(ImportError::LiveLeases { leases }) => {
            eprintln!(
                "the catalog holds {} live lease(s); a replace does not overwrite them:",
                leases.len()
            );
            for (key, provenance) in &leases {
                if let Provenance::Lease { writer, expires_at } = provenance {
                    eprintln!(
                        "{}",
                        DisplayText::new(&format!(
                            "  {}  {writer}  live until {expires_at}",
                            key_text(key)
                        ))
                    );
                }
            }
            FAILED
        }
        Err(e) => fail(&e.to_string()),
    }
}

async fn purge(target: Target, yes: bool) -> u8 {
    // A removal must know the served id to refuse it; an unset variable would let any id through.
    match std::env::var(LIVE_CATALOG_ID) {
        Err(_) if yes => {
            return fail(&format!(
                "refusing to purge catalog id {} with --yes while {LIVE_CATALOG_ID} is unset: set \
                 it to the id served by the deployed function",
                target.catalog_id
            ));
        }
        Ok(live) if live == target.catalog_id => {
            return fail(&format!(
                "refusing to purge catalog id {}: it is the value of {LIVE_CATALOG_ID}, the id \
                 served by the deployed function; give the id to purge as --catalog-id",
                target.catalog_id
            ));
        }
        _ => {}
    }
    let (store, catalog) = match target.open().await {
        Ok(opened) => opened,
        Err(message) => return fail(&message),
    };
    let counts = if yes {
        store.purge_catalog(&catalog).await.map(|r| r.removed)
    } else {
        store.count_catalog(&catalog).await
    };
    match counts {
        Ok(counts) => {
            print_counts(&counts);
            if yes {
                println!("removed {} items", counts.total());
            } else {
                println!("nothing removed; run again with --yes to remove");
            }
            OK
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn print_counts(counts: &ItemCounts) {
    println!("host {}", counts.host);
    println!("entries {}", counts.entries);
    println!("guards {}", counts.guards);
    println!("other {}", counts.other);
}

async fn execute(command: Loaded) -> u8 {
    let command = match command {
        Loaded::Import {
            target,
            document,
            writer,
            replace,
        } => return import_document(target, *document, writer, replace).await,
        Loaded::Other(command) => command,
    };
    match command {
        StoreCommand::Import { .. } => unreachable!("an import is loaded before the runtime"),
        StoreCommand::Purge { target, yes } => purge(target, yes).await,
        StoreCommand::List { target } => {
            let (store, catalog) = match target.open().await {
                Ok(opened) => opened,
                Err(message) => return fail(&message),
            };
            match list_registrations(&*store, &catalog, SystemTime::now()).await {
                Ok(rows) => {
                    let mut out = io::stdout().lock();
                    let written = rows
                        .iter()
                        .try_for_each(|row| writeln!(out, "{}", DisplayText::new(&row.to_string())))
                        .and_then(|()| out.flush());
                    if written.is_ok() { OK } else { FAILED }
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        StoreCommand::Remove {
            target,
            identifier,
            version,
        } => {
            let (store, catalog) = match target.open().await {
                Ok(opened) => opened,
                Err(message) => return fail(&message),
            };
            let key = EntryKey::new(EntryIdentifier::new(identifier), version);
            match remove_registration(&*store, &catalog, &key).await {
                Ok(_) => {
                    println!("removed {}", DisplayText::new(&key_text(&key)));
                    OK
                }
                Err(e) => fail(&format!("{}: {e}", key_text(&key))),
            }
        }
    }
}

fn key_text(key: &EntryKey) -> String {
    match key.version() {
        Some(version) => format!("{}@{version}", key.identifier().as_str()),
        None => key.identifier().as_str().to_string(),
    }
}
