//! The signing key held in AWS Systems Manager Parameter Store as a `SecureString`.

use std::fmt;

use aws_sdk_ssm::Client;
use aws_sdk_ssm::error::DisplayErrorContext;
use aws_sdk_ssm::types::{ParameterTier, ParameterType};

use crate::sign::SigningKey;

/// Reports why the key could not be created in or loaded from Parameter Store.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyStoreError {
    /// The parameter exists; a key is never replaced by this crate.
    AlreadyExists,
    /// The parameter does not exist.
    NotFound,
    /// The parameter is not a SecureString, or its value is not a P-256 PKCS#8 PEM.
    Malformed,
    /// Any other service error, as its message.
    Service(String),
}

impl fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => f.write_str("the parameter already exists"),
            Self::NotFound => f.write_str("the parameter does not exist"),
            Self::Malformed => {
                f.write_str("the parameter is not a SecureString holding a P-256 PKCS#8 PEM")
            }
            Self::Service(message) => write!(f, "Parameter Store failed: {message}"),
        }
    }
}

impl std::error::Error for KeyStoreError {}

/// The signing key of a publisher at rest: one `SecureString` parameter, encrypted with the
/// AWS-managed `aws/ssm` key, holding the PKCS#8 PEM.
///
/// The publishing role needs `ssm:PutParameter` (key creation only) and `ssm:GetParameter` on
/// the parameter. The PEM is held only in memory; nothing here writes a file or prints a key.
#[derive(Clone, Debug)]
pub struct ParameterKeyStore {
    client: Client,
    name: String,
}

impl ParameterKeyStore {
    /// Returns the store for parameter `name`; `client` is injected, so its configuration,
    /// credentials and endpoint are the caller's.
    pub fn new(client: Client, name: String) -> Self {
        Self { client, name }
    }

    /// Writes `key` as a new parameter and never replaces one.
    ///
    /// # Errors
    ///
    /// - [`KeyStoreError::AlreadyExists`] when the parameter exists; nothing is written.
    /// - [`KeyStoreError::Malformed`] when `key` is not a P-256 PKCS#8 key.
    /// - [`KeyStoreError::Service`] for any other service error.
    pub async fn create(&self, key: &SigningKey) -> Result<(), KeyStoreError> {
        let pem = key.to_pem().map_err(|_| KeyStoreError::Malformed)?;
        self.client
            .put_parameter()
            .name(&self.name)
            .value(pem)
            .r#type(ParameterType::SecureString)
            .overwrite(false)
            .tier(ParameterTier::Standard)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| {
                if e.as_service_error()
                    .is_some_and(|s| s.is_parameter_already_exists())
                {
                    KeyStoreError::AlreadyExists
                } else {
                    KeyStoreError::Service(DisplayErrorContext(&e).to_string())
                }
            })
    }

    /// Reads and decrypts the parameter.
    ///
    /// # Errors
    ///
    /// - [`KeyStoreError::NotFound`] when the parameter does not exist.
    /// - [`KeyStoreError::Malformed`] when the parameter is not a SecureString or its value is
    ///   not a P-256 PKCS#8 PEM.
    /// - [`KeyStoreError::Service`] for any other service error.
    pub async fn load(&self) -> Result<SigningKey, KeyStoreError> {
        let output = self
            .client
            .get_parameter()
            .name(&self.name)
            .with_decryption(true)
            .send()
            .await
            .map_err(|e| {
                if e.as_service_error()
                    .is_some_and(|s| s.is_parameter_not_found())
                {
                    KeyStoreError::NotFound
                } else {
                    KeyStoreError::Service(DisplayErrorContext(&e).to_string())
                }
            })?;
        let parameter = output.parameter().ok_or(KeyStoreError::Malformed)?;
        // Only a SecureString was encrypted at rest; a key found in any other type is refused.
        if parameter.r#type() != Some(&ParameterType::SecureString) {
            return Err(KeyStoreError::Malformed);
        }
        let value = parameter.value().ok_or(KeyStoreError::Malformed)?;
        SigningKey::from_pem(value).map_err(|_| KeyStoreError::Malformed)
    }
}
