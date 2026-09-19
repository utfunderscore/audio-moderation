//! Shared types and utilities for the workspace.

use std::env;
use std::io::Error as IoError;

use aws_sdk_ssm::Client as SsmClient;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Loads a decrypted SSM parameter named by an environment variable.
pub async fn load_secure_parameter(
    ssm_client: &SsmClient,
    parameter_environment_variable: &str,
) -> Result<String, Error> {
    let parameter_name = env::var(parameter_environment_variable)
        .unwrap_or_else(|_| panic!("{parameter_environment_variable} must be set"));
    let response = ssm_client
        .get_parameter()
        .name(parameter_name)
        .with_decryption(true)
        .send()
        .await?;

    response
        .parameter()
        .and_then(|parameter| parameter.value())
        .map(str::to_owned)
        .ok_or_else(|| IoError::other("secure parameter has no value").into())
}

/// Loads the database connection URL from the encrypted SSM parameter named by
/// `DATABASE_URL_PARAMETER`.
pub async fn load_database_url(ssm_client: &SsmClient) -> Result<String, Error> {
    load_secure_parameter(ssm_client, "DATABASE_URL_PARAMETER").await
}

#[derive(Clone)]
pub struct EvaluationAccess {
    secret: Vec<u8>,
}

impl EvaluationAccess {
    pub fn new(secret: String) -> Result<Self, Error> {
        if secret.as_bytes().len() < 32 {
            return Err(
                IoError::other("evaluation access secret must contain at least 32 bytes").into(),
            );
        }
        Ok(Self {
            secret: secret.into_bytes(),
        })
    }

    pub fn token(&self, evaluation_id: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret)
            .expect("HMAC accepts keys of any non-zero length");
        mac.update(b"audio-moderation/evaluation-access/v1\0");
        mac.update(evaluation_id.as_bytes());
        format!(
            "eval_v1.{}",
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        )
    }

    pub fn verify(&self, evaluation_id: &str, token: &str) -> bool {
        let Some(encoded_mac) = token.strip_prefix("eval_v1.") else {
            return false;
        };
        let Ok(provided_mac) = URL_SAFE_NO_PAD.decode(encoded_mac) else {
            return false;
        };
        let mut expected = Hmac::<Sha256>::new_from_slice(&self.secret)
            .expect("HMAC accepts keys of any non-zero length");
        expected.update(b"audio-moderation/evaluation-access/v1\0");
        expected.update(evaluation_id.as_bytes());
        expected.verify_slice(&provided_mac).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::EvaluationAccess;

    #[test]
    fn evaluation_access_tokens_are_stable_and_resource_scoped() {
        let access = EvaluationAccess::new("a sufficiently long test secret value".into()).unwrap();
        let token = access.token("evaluation-a");

        assert_eq!(token, access.token("evaluation-a"));
        assert!(access.verify("evaluation-a", &token));
        assert!(!access.verify("evaluation-b", &token));
        assert!(!access.verify("evaluation-a", "eval_v1.invalid"));
    }
}
