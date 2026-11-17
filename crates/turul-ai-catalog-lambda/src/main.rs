//! The Lambda function: serves the catalog assembled from the configured store.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use lambda_http::{Error, run, service_fn};
use turul_ai_catalog_lambda::{
    ID_VAR, MAX_AGE_VAR, ServedCatalog, SnapshotCache, catalog_id, handle, max_age,
};
use turul_ai_catalog_storage::CatalogStore;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[cfg(feature = "dynamodb")]
async fn backend() -> Result<(Arc<dyn CatalogStore>, turul_ai_catalog_storage::CatalogId), Error> {
    use turul_ai_catalog_lambda::{TABLE_VAR, table_name};
    let table = table_name(var(TABLE_VAR).as_deref())?;
    let catalog = catalog_id(var(ID_VAR).as_deref())?;
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_dynamodb::Client::new(&config);
    let store = turul_ai_catalog_storage::DynamoDbCatalogStore::new(client, table);
    Ok((Arc::new(store), catalog))
}

#[cfg(not(feature = "dynamodb"))]
async fn backend() -> Result<(Arc<dyn CatalogStore>, turul_ai_catalog_storage::CatalogId), Error> {
    let catalog = catalog_id(Some(var(ID_VAR).as_deref().unwrap_or("default")))?;
    Ok((
        Arc::new(turul_ai_catalog_storage::InMemoryCatalogStore::new()),
        catalog,
    ))
}

fn report_omissions(served: &ServedCatalog) {
    for omission in served.omitted() {
        eprintln!("{omission}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let max_age = max_age(var(MAX_AGE_VAR).as_deref())?;
    let (store, catalog) = backend().await?;
    let first = ServedCatalog::assemble(&*store, &catalog, SystemTime::now()).await?;
    report_omissions(&first);
    let cache = SnapshotCache::new(first, max_age, SystemTime::now);
    let (store, catalog, cache) = (&store, &catalog, &cache);
    let reported: Mutex<Option<Arc<ServedCatalog>>> = Mutex::new(None);
    let reported = &reported;
    run(service_fn(move |request| async move {
        let (served, refresh_error) = cache.current(&**store, catalog).await;
        // One line per omission per assembly: a snapshot is reported once, when it first appears.
        let mut last = reported.lock().unwrap_or_else(|p| p.into_inner());
        if last.as_ref().is_none_or(|l| !Arc::ptr_eq(l, &served)) {
            if last.is_some() {
                report_omissions(&served);
            }
            *last = Some(Arc::clone(&served));
        }
        drop(last);
        if let Some(e) = refresh_error {
            eprintln!(
                "catalog refresh failed, serving the last good snapshot: {}",
                turul_ai_catalog::error::chain(&e)
            );
        }
        Ok::<_, Error>(handle(&served, &request))
    }))
    .await
}
