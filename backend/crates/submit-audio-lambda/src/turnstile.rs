use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::Deserialize;

const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
const SITEVERIFY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 4096;
pub(crate) const MAX_TOKEN_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyError {
    Rejected,
    Unavailable,
}

pub(crate) trait TurnstileVerifier: Send + Sync {
    fn verify<'a>(
        &'a self,
        token: &'a str,
        idempotency_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VerifyError>> + Send + 'a>>;
}

#[derive(Clone)]
pub(crate) struct Siteverify {
    client: reqwest::Client,
    secret: String,
    allowed_hostnames: HashSet<String>,
    expected_action: String,
    url: String,
}

#[derive(Deserialize)]
struct SiteverifyResponse {
    success: bool,
    hostname: Option<String>,
    action: Option<String>,
}

impl Siteverify {
    pub(crate) fn new(
        secret: String,
        allowed_hostnames: &str,
        expected_action: &str,
    ) -> Result<Self, &'static str> {
        if secret.trim().is_empty() {
            return Err("Turnstile secret must not be empty");
        }
        let hostnames: Vec<_> = allowed_hostnames.split(',').map(str::trim).collect();
        if hostnames.iter().any(|host| {
            host.is_empty()
                || !host.is_ascii()
                || host
                    .chars()
                    .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '.'))
                || host.starts_with('-')
                || host.contains("..")
        }) {
            return Err("TURNSTILE_ALLOWED_HOSTNAMES must contain exact hostnames");
        }
        let expected_action = expected_action.trim();
        if expected_action.is_empty() {
            return Err("Turnstile expected action must not be empty");
        }
        let client = reqwest::Client::builder()
            .timeout(SITEVERIFY_TIMEOUT)
            // Never forward the secret or response token to a redirect target.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "failed to configure Turnstile HTTP client")?;
        Ok(Self {
            client,
            secret,
            allowed_hostnames: hostnames.into_iter().map(str::to_ascii_lowercase).collect(),
            expected_action: expected_action.to_owned(),
            url: SITEVERIFY_URL.to_owned(),
        })
    }

    async fn verify_token(&self, token: &str, idempotency_key: &str) -> Result<(), VerifyError> {
        let mut response = self
            .client
            .post(&self.url)
            .form(&[
                ("secret", self.secret.as_str()),
                ("response", token),
                // Cloudflare deduplicates verification retries for this submission UUID.
                ("idempotency_key", idempotency_key),
            ])
            .send()
            .await
            .map_err(|_| VerifyError::Unavailable)?;
        if !response.status().is_success() {
            return Err(VerifyError::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| VerifyError::Unavailable)?
        {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(VerifyError::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: SiteverifyResponse =
            serde_json::from_slice(&bytes).map_err(|_| VerifyError::Unavailable)?;
        if result.success
            && result.hostname.as_deref().is_some_and(|hostname| {
                self.allowed_hostnames
                    .contains(&hostname.to_ascii_lowercase())
            })
            && result.action.as_deref() == Some(self.expected_action.as_str())
        {
            Ok(())
        } else {
            Err(VerifyError::Rejected)
        }
    }
}

impl TurnstileVerifier for Siteverify {
    fn verify<'a>(
        &'a self,
        token: &'a str,
        idempotency_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VerifyError>> + Send + 'a>> {
        Box::pin(self.verify_token(token, idempotency_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(body: &'static str, status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/siteverify", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0 && request.len() + count <= 4096);
                request.extend_from_slice(&chunk[..count]);
                if request
                    .windows(b"idempotency_key=".len())
                    .any(|part| part == b"idempotency_key=")
                {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /siteverify HTTP/1.1"));
            assert!(request.contains("secret=test-secret"));
            assert!(request.contains("response=test-token"));
            assert!(request.contains("idempotency_key="));
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        url
    }

    #[tokio::test]
    async fn verifies_success_hostname_and_action_and_fails_closed() {
        let mut verifier = Siteverify::new(
            "test-secret".into(),
            "app.example, localhost",
            "submit_review",
        )
        .unwrap();
        for (body, status, expected) in [
            (
                r#"{"success":true,"hostname":"app.example","action":"submit_review"}"#,
                "200 OK",
                Ok(()),
            ),
            (
                r#"{"success":false,"hostname":"app.example","action":"submit_review"}"#,
                "200 OK",
                Err(VerifyError::Rejected),
            ),
            (
                r#"{"success":true,"hostname":"evil.app.example","action":"submit_review"}"#,
                "200 OK",
                Err(VerifyError::Rejected),
            ),
            (
                r#"{"success":true,"hostname":"app.example","action":"other"}"#,
                "200 OK",
                Err(VerifyError::Rejected),
            ),
            (r#"{"success":true}"#, "200 OK", Err(VerifyError::Rejected)),
            (
                r#"{"success":true,"hostname":"app.example"}"#,
                "200 OK",
                Err(VerifyError::Rejected),
            ),
            ("not-json", "200 OK", Err(VerifyError::Unavailable)),
            (
                "{}",
                "503 Service Unavailable",
                Err(VerifyError::Unavailable),
            ),
            ("{}", "302 Found", Err(VerifyError::Unavailable)),
        ] {
            verifier.url = server(body, status).await;
            assert_eq!(
                verifier
                    .verify("test-token", "44444444-4444-4444-4444-444444444444")
                    .await,
                expected
            );
        }
    }

    #[test]
    fn rejects_insecure_configuration() {
        assert!(Siteverify::new(" ".into(), "localhost", "submit_review").is_err());
        assert!(Siteverify::new("secret".into(), "", "submit_review").is_err());
        assert!(Siteverify::new("secret".into(), "localhost,", "submit_review").is_err());
        assert!(Siteverify::new("secret".into(), "*.example.com", "submit_review").is_err());
        assert!(Siteverify::new("secret".into(), "localhost", "  ").is_err());
    }
}
