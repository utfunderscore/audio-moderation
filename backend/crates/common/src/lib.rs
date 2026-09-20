//! Shared types and utilities for the workspace.

use std::env;
use std::io::Error as IoError;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// A short-lived bearer capability scoped to one tenant's review job.
#[derive(Clone)]
pub struct ReviewAccess {
    secret: Vec<u8>,
}

const REVIEW_ACCESS_PREFIX: &str = "review_v1.";
const REVIEW_ACCESS_DOMAIN: &[u8] = b"audio-moderation/review-access/v1\0";
const REVIEW_ACCESS_SCOPE: &str = "review:read evaluation:read task-events:create";

impl ReviewAccess {
    pub fn new(secret: String) -> Result<Self, Error> {
        if secret.as_bytes().len() < 32 {
            return Err(
                IoError::other("review access secret must contain at least 32 bytes").into(),
            );
        }
        Ok(Self {
            secret: secret.into_bytes(),
        })
    }

    pub fn token(&self, tenant_id: &str, review_id: i32, expires_at: SystemTime) -> String {
        let expires_at = expires_at
            .duration_since(UNIX_EPOCH)
            .expect("review capability expiry must be after the Unix epoch")
            .as_secs();
        let payload = format!("{tenant_id}\n{review_id}\n{expires_at}\n{REVIEW_ACCESS_SCOPE}");
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let signature = self.sign(encoded_payload.as_bytes());
        format!(
            "{REVIEW_ACCESS_PREFIX}{encoded_payload}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        )
    }

    pub fn verify(&self, tenant_id: &str, review_id: i32, token: &str, now: SystemTime) -> bool {
        self.authenticated_review(tenant_id, token, now) == Some(review_id)
    }

    /// Returns the scoped review ID only after signature, tenant, scope, and expiry checks.
    pub fn authenticated_review(
        &self,
        tenant_id: &str,
        token: &str,
        now: SystemTime,
    ) -> Option<i32> {
        let Some(value) = token.strip_prefix(REVIEW_ACCESS_PREFIX) else {
            return None;
        };
        let Some((encoded_payload, encoded_signature)) = value.split_once('.') else {
            return None;
        };
        let Ok(signature) = URL_SAFE_NO_PAD.decode(encoded_signature) else {
            return None;
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret)
            .expect("HMAC accepts keys of any non-zero length");
        mac.update(REVIEW_ACCESS_DOMAIN);
        mac.update(encoded_payload.as_bytes());
        if mac.verify_slice(&signature).is_err() {
            return None;
        }

        let Ok(payload) = URL_SAFE_NO_PAD.decode(encoded_payload) else {
            return None;
        };
        let Ok(payload) = std::str::from_utf8(&payload) else {
            return None;
        };
        let mut fields = payload.split('\n');
        let (Some(token_tenant), Some(token_review), Some(token_expiry), Some(scope), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return None;
        };
        let Ok(token_review) = token_review.parse::<i32>() else {
            return None;
        };
        let Ok(token_expiry) = token_expiry.parse::<u64>() else {
            return None;
        };
        let Ok(now) = now.duration_since(UNIX_EPOCH) else {
            return None;
        };

        (token_tenant == tenant_id && scope == REVIEW_ACCESS_SCOPE && now.as_secs() < token_expiry)
            .then_some(token_review)
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret)
            .expect("HMAC accepts keys of any non-zero length");
        mac.update(REVIEW_ACCESS_DOMAIN);
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::ReviewAccess;

    #[test]
    fn review_access_tokens_are_expiring_tenant_and_resource_scoped() {
        let access = ReviewAccess::new("a sufficiently long test secret value".into()).unwrap();
        let expiry = UNIX_EPOCH + Duration::from_secs(2_000);
        let token = access.token("tenant-a", 42, expiry);

        assert!(access.verify(
            "tenant-a",
            42,
            &token,
            UNIX_EPOCH + Duration::from_secs(1_999)
        ));
        assert!(!access.verify(
            "tenant-b",
            42,
            &token,
            UNIX_EPOCH + Duration::from_secs(1_999)
        ));
        assert!(!access.verify(
            "tenant-a",
            43,
            &token,
            UNIX_EPOCH + Duration::from_secs(1_999)
        ));
        assert!(!access.verify("tenant-a", 42, &token, expiry));
        assert!(!access.verify("tenant-a", 42, &format!("{token}x"), UNIX_EPOCH));
    }
}
