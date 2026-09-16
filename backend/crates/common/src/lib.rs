//! Shared types and utilities for the workspace.

use std::env;
use std::io::Error as IoError;

use aws_sdk_ssm::Client as SsmClient;

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
