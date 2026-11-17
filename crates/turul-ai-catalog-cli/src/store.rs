//! The `store` command: arguments, backend connection and printing. Listing and removal are
//! `turul_ai_catalog_storage::list_registrations` and `remove_registration`.

use std::io::{self, Write};
use std::time::SystemTime;

use clap::{Args, Subcommand};
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog_client::DisplayText;
use turul_ai_catalog_storage::{CatalogId, CatalogStore, list_registrations, remove_registration};

use crate::{FAILED, OK, fail};

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
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return fail(&e.to_string()),
    };
    runtime.block_on(execute(command))
}

async fn execute(command: StoreCommand) -> u8 {
    match command {
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
