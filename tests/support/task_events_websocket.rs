//! Test-only client for the task-events WebSocket transport.
//!
//! Task events are plain UTF-8 event-name frames. Delivery is at least once,
//! so callers must tolerate duplicate names at the replay/live boundary.

use std::io::Error as IoError;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::time::timeout;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error as TungsteniteError, Message},
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A single-task task-events WebSocket subscription.
pub struct TaskEventsWebSocket {
    stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl TaskEventsWebSocket {
    /// Connects and subscribes this socket to exactly one pipeline task.
    pub async fn connect_and_subscribe(
        endpoint: &str,
        task_id: i32,
        connect_timeout: Duration,
    ) -> Result<Self, Error> {
        ensure_rustls_crypto_provider();
        let (mut stream, _) = timeout(connect_timeout, connect_async(endpoint))
            .await
            .map_err(|_| timeout_error("WebSocket connection", connect_timeout))??;
        let subscription = json!({ "action": "subscribe", "taskId": task_id }).to_string();
        timeout(
            connect_timeout,
            stream.send(Message::Text(subscription.into())),
        )
        .await
        .map_err(|_| timeout_error("WebSocket subscription", connect_timeout))??;
        Ok(Self { stream })
    }

    /// Waits for the next UTF-8 event-name frame while servicing WebSocket control frames.
    pub async fn next_event(&mut self, wait: Duration) -> Result<String, Error> {
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| timeout_error("WebSocket event", wait))?;
            let frame = timeout(remaining, self.stream.next())
                .await
                .map_err(|_| timeout_error("WebSocket event", wait))?
                .ok_or_else(|| IoError::other("WebSocket closed before an event arrived"))??;
            match frame {
                Message::Text(event) => return Ok(event.to_string()),
                Message::Binary(event) => {
                    return String::from_utf8(event.to_vec()).map_err(|error| {
                        IoError::new(std::io::ErrorKind::InvalidData, error).into()
                    });
                }
                Message::Ping(payload) => {
                    let remaining = deadline
                        .checked_duration_since(Instant::now())
                        .ok_or_else(|| timeout_error("WebSocket event", wait))?;
                    timeout(remaining, self.stream.send(Message::Pong(payload)))
                        .await
                        .map_err(|_| timeout_error("WebSocket event", wait))??;
                }
                Message::Pong(_) => {}
                Message::Close(_) => {
                    // Tungstenite queues the close reply while reading. Flush it
                    // before reporting that no further event can arrive.
                    let _ = timeout(CLOSE_TIMEOUT, self.stream.flush()).await;
                    return Err(IoError::other("WebSocket peer closed the connection").into());
                }
                Message::Frame(_) => {}
            }
        }
    }

    /// Starts a close handshake, services control frames, and then releases the socket.
    pub async fn close_cleanly(mut self) -> Result<(), Error> {
        match timeout(CLOSE_TIMEOUT, self.stream.send(Message::Close(None))).await {
            Ok(Ok(())) | Ok(Err(TungsteniteError::ConnectionClosed)) => {}
            Ok(Err(error)) => return Err(error.into()),
            // Dropping after a bounded close attempt is intentional for a test client.
            Err(_) => return Ok(()),
        }

        loop {
            match timeout(CLOSE_TIMEOUT, self.stream.next()).await {
                Ok(Some(Ok(Message::Ping(payload)))) => {
                    self.stream.send(Message::Pong(payload)).await?
                }
                Ok(Some(Ok(Message::Close(_)))) => {
                    let _ = timeout(CLOSE_TIMEOUT, self.stream.flush()).await;
                    return Ok(());
                }
                Ok(None) | Err(_) => return Ok(()),
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(TungsteniteError::ConnectionClosed))) => return Ok(()),
                Ok(Some(Err(error))) => return Err(error.into()),
            }
        }
    }
}

fn ensure_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // The deployed-test dependency graph enables AWS-LC through the AWS SDK
        // and Ring through SQLx. Rustls cannot choose between both features, so
        // the test client must select one before Tokio-Tungstenite builds TLS.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

fn timeout_error(operation: &str, wait: Duration) -> IoError {
    IoError::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "{operation} did not complete within {} seconds",
            wait.as_secs()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::ensure_rustls_crypto_provider;

    #[test]
    fn rustls_provider_can_be_selected_for_websocket_tls() {
        ensure_rustls_crypto_provider();
        let _ = rustls::ClientConfig::builder();
    }
}
