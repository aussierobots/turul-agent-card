#![cfg(feature = "ssm")]
//! The signing key in Parameter Store: requests sent and answers handled. A loopback endpoint
//! answers with canned responses and an interceptor records each request's target and body.

mod support;

use std::sync::{Arc, Mutex};

use aws_sdk_ssm::Client;
use aws_sdk_ssm::config::interceptors::BeforeTransmitInterceptorContextRef;
use aws_sdk_ssm::config::{
    Config, ConfigBag, Credentials, Intercept, Region, RuntimeComponents, retry::RetryConfig,
};
use aws_sdk_ssm::error::BoxError;
use support::fake_ssm::FakeSsm;
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog_trust::{KeyStoreError, ParameterKeyStore, SigningKey};

const NAME: &str = "/publisher/signing-key";

type Seen = Arc<Mutex<Vec<(String, Json)>>>;

#[derive(Debug)]
struct Recorder(Seen);

impl Intercept for Recorder {
    fn name(&self) -> &'static str {
        "Recorder"
    }

    fn read_before_transmit(
        &self,
        context: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let request = context.request();
        let target = request
            .headers()
            .get("x-amz-target")
            .unwrap_or_default()
            .to_string();
        let body = request.body().bytes().expect("a buffered body");
        self.0
            .lock()
            .unwrap()
            .push((target, read(body).expect("a JSON body")));
        Ok(())
    }
}

/// A store over `endpoint` without SDK retries; `seen` records each request it sends.
fn store(endpoint: &FakeSsm) -> (ParameterKeyStore, Seen) {
    let seen = Seen::default();
    let config = Config::builder()
        .behavior_version_latest()
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("id", "secret", None, None, "test"))
        .endpoint_url(endpoint.endpoint())
        .retry_config(RetryConfig::disabled())
        .interceptor(Recorder(seen.clone()))
        .build();
    (
        ParameterKeyStore::new(Client::from_conf(config), NAME.to_string()),
        seen,
    )
}

fn member<'a>(body: &'a Json, name: &str) -> Option<&'a Json> {
    match body {
        Json::Object(m) => m.get(name),
        other => panic!("not an object: {other:?}"),
    }
}

fn text(value: &Json) -> &str {
    match value {
        Json::String(s) => s,
        other => panic!("not a string: {other:?}"),
    }
}

#[tokio::test]
async fn create_puts_a_secure_string_without_overwrite() {
    let ssm = FakeSsm::start();
    let (store, seen) = store(&ssm);
    store.create(&SigningKey::generate()).await.unwrap();

    let seen = seen.lock().unwrap();
    let [(target, body)] = seen.as_slice() else {
        panic!("one request expected: {seen:?}");
    };
    assert_eq!(target, "AmazonSSM.PutParameter");
    assert_eq!(text(member(body, "Name").unwrap()), NAME);
    assert_eq!(text(member(body, "Type").unwrap()), "SecureString");
    assert_eq!(member(body, "Overwrite"), Some(&Json::Bool(false)));
    assert_eq!(text(member(body, "Tier").unwrap()), "Standard");
    assert!(member(body, "KeyId").is_none());
    let value = text(member(body, "Value").unwrap());
    assert!(value.starts_with("-----BEGIN PRIVATE KEY-----"));
    assert_eq!(ssm.value(NAME).as_deref(), Some(value));
}

#[tokio::test]
async fn existing_parameter_is_refused() {
    let ssm = FakeSsm::start();
    ssm.seed(NAME, "original");
    let (store, _) = store(&ssm);
    let refused = store.create(&SigningKey::generate()).await;
    assert_eq!(refused, Err(KeyStoreError::AlreadyExists));
    assert_eq!(
        ssm.value(NAME).as_deref(),
        Some("original"),
        "nothing written"
    );
}

#[tokio::test]
async fn load_decrypts_and_parses() {
    let ssm = FakeSsm::start();
    let (store, seen) = store(&ssm);
    let key = SigningKey::generate();
    store.create(&key).await.unwrap();
    seen.lock().unwrap().clear();

    let loaded = store.load().await.unwrap();
    assert_eq!(loaded.public_jwk().unwrap(), key.public_jwk().unwrap());

    let seen = seen.lock().unwrap();
    let [(target, body)] = seen.as_slice() else {
        panic!("one request expected: {seen:?}");
    };
    assert_eq!(target, "AmazonSSM.GetParameter");
    assert_eq!(text(member(body, "Name").unwrap()), NAME);
    assert_eq!(member(body, "WithDecryption"), Some(&Json::Bool(true)));
}

#[tokio::test]
async fn missing_or_malformed_parameter_is_refused() {
    let ssm = FakeSsm::start();
    let (store, _) = store(&ssm);
    assert_eq!(store.load().await.unwrap_err(), KeyStoreError::NotFound);

    ssm.seed(NAME, "not a pem");
    assert_eq!(store.load().await.unwrap_err(), KeyStoreError::Malformed);

    ssm.seed(
        NAME,
        "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n",
    );
    assert_eq!(store.load().await.unwrap_err(), KeyStoreError::Malformed);

    // A key stored in a plain `String` parameter was never encrypted at rest: refused.
    ssm.state.lock().unwrap().parameters.remove(NAME);
    let fresh = SigningKey::generate();
    store.create(&fresh).await.unwrap();
    assert!(store.load().await.is_ok());
    ssm.state.lock().unwrap().plain.insert(NAME.to_string());
    assert_eq!(store.load().await.unwrap_err(), KeyStoreError::Malformed);

    ssm.state.lock().unwrap().deny = true;
    assert!(matches!(
        store.load().await.unwrap_err(),
        KeyStoreError::Service(_)
    ));
}
