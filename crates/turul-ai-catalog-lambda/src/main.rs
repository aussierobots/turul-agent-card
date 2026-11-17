use lambda_http::{Error, run, service_fn};
use turul_ai_catalog_lambda::{BUNDLE, ServedCatalog, handle};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let served = ServedCatalog::load(BUNDLE)?;
    let served = &served;
    run(service_fn(move |request| async move {
        Ok::<_, Error>(handle(served, &request))
    }))
    .await
}
