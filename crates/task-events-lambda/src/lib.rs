use std::future::Future;

use database::{
    NewPipelineTaskWebSocketConnection, PipelineTaskWebSocketConnectionError,
    PipelineTaskWebSocketConnectionStore,
};
use lambda_runtime::{Error, LambdaEvent};
use serde::{Deserialize, Serialize};
use task_event_emitter::TaskEventEmitter;
use tracing::info;

/// The API Gateway WebSocket event received by this Lambda.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiGatewayWebSocketEvent {
    pub request_context: WebSocketRequestContext,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub is_base64_encoded: bool,
}

/// WebSocket-specific fields supplied by API Gateway for every route invocation.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketRequestContext {
    pub connection_id: ConnectionId,
    pub route_key: WebSocketRoute,
}

/// An API Gateway WebSocket connection identifier.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ConnectionId(pub String);

/// Routes handled by the task-events WebSocket API.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum WebSocketRoute {
    #[serde(rename = "$connect")]
    Connect,
    #[serde(rename = "subscribe")]
    Subscribe,
    #[serde(rename = "$disconnect")]
    Disconnect,
}

/// Input for the `$connect` route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectEvent {
    pub connection_id: ConnectionId,
}

/// Input accepted by the `subscribe` route.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeRequest {
    pub task_id: i32,
}

/// Input for the `subscribe` route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscribeEvent {
    pub connection_id: ConnectionId,
    pub request: SubscribeRequest,
}

/// Input for the `$disconnect` route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisconnectEvent {
    pub connection_id: ConnectionId,
}

/// The Lambda proxy response returned to API Gateway.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketResponse {
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Handles incoming task-events WebSocket route invocations.
#[derive(Clone)]
pub struct TaskEventsHandler<S> {
    connections: S,
    events: Option<TaskEventEmitter>,
}

/// Persistence required by the task-events WebSocket handler.
pub trait TaskConnectionStore: Send + Sync {
    fn subscribe(
        &self,
        task_id: i32,
        connection_id: String,
    ) -> impl Future<Output = Result<(), PipelineTaskWebSocketConnectionError>> + Send;

    fn remove(
        &self,
        connection_id: String,
    ) -> impl Future<Output = Result<u64, PipelineTaskWebSocketConnectionError>> + Send;
}

impl TaskConnectionStore for PipelineTaskWebSocketConnectionStore {
    async fn subscribe(
        &self,
        task_id: i32,
        connection_id: String,
    ) -> Result<(), PipelineTaskWebSocketConnectionError> {
        self.create_or_get(NewPipelineTaskWebSocketConnection {
            task_id,
            connection_id: &connection_id,
        })
        .await
        .map(|_| ())
    }

    async fn remove(
        &self,
        connection_id: String,
    ) -> Result<u64, PipelineTaskWebSocketConnectionError> {
        PipelineTaskWebSocketConnectionStore::remove(self, &connection_id).await
    }
}

impl<S> TaskEventsHandler<S> {
    pub fn new(connections: S) -> Self {
        Self {
            connections,
            events: None,
        }
    }

    pub fn with_event_emitter(mut self, events: TaskEventEmitter) -> Self {
        self.events = Some(events);
        self
    }
}

impl<S: TaskConnectionStore> TaskEventsHandler<S> {
    /// Dispatches an API Gateway WebSocket invocation to its route handler.
    pub async fn handle(
        &self,
        event: LambdaEvent<ApiGatewayWebSocketEvent>,
    ) -> Result<WebSocketResponse, Error> {
        let ApiGatewayWebSocketEvent {
            request_context,
            body,
            ..
        } = event.payload;
        let connection_id = request_context.connection_id;

        match request_context.route_key {
            WebSocketRoute::Connect => self.connect(ConnectEvent { connection_id }).await,
            WebSocketRoute::Subscribe => {
                let body =
                    body.ok_or_else(|| invalid_input("subscribe requests require a body"))?;
                let request = serde_json::from_str(&body)?;
                self.subscribe(SubscribeEvent {
                    connection_id,
                    request,
                })
                .await
            }
            WebSocketRoute::Disconnect => self.disconnect(DisconnectEvent { connection_id }).await,
        }
    }

    /// Logs a newly accepted WebSocket connection.
    pub async fn connect(&self, _event: ConnectEvent) -> Result<WebSocketResponse, Error> {
        Ok(success_response())
    }

    /// Logs a requested task-event subscription.
    pub async fn subscribe(&self, event: SubscribeEvent) -> Result<WebSocketResponse, Error> {
        self.connections
            .subscribe(event.request.task_id, event.connection_id.0.clone())
            .await?;
        if let Some(events) = &self.events {
            let replay = events
                .replay(event.request.task_id, &event.connection_id.0)
                .await?;
            info!(
                connectionId = %event.connection_id.0,
                taskId = event.request.task_id,
                replayedEvents = replay.delivered,
                removedStaleConnections = replay.removed_stale_connections,
                "replayed WebSocket task events"
            );
        }
        info!(
            connectionId = %event.connection_id.0,
            taskId = event.request.task_id,
            "registered WebSocket task subscription"
        );
        Ok(success_response())
    }

    /// Removes every subscription held by a disconnected WebSocket connection.
    pub async fn disconnect(&self, event: DisconnectEvent) -> Result<WebSocketResponse, Error> {
        let removed = self
            .connections
            .remove(event.connection_id.0.clone())
            .await?;
        info!(connectionId = %event.connection_id.0, removedSubscriptions = removed, "removed WebSocket subscriptions");
        Ok(success_response())
    }
}

fn success_response() -> WebSocketResponse {
    WebSocketResponse {
        status_code: 200,
        body: None,
    }
}

fn invalid_input(message: &'static str) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingConnectionStore {
        subscriptions: Arc<Mutex<Vec<(i32, String)>>>,
        removals: Arc<Mutex<Vec<String>>>,
    }

    impl TaskConnectionStore for RecordingConnectionStore {
        async fn subscribe(
            &self,
            task_id: i32,
            connection_id: String,
        ) -> Result<(), PipelineTaskWebSocketConnectionError> {
            self.subscriptions
                .lock()
                .unwrap()
                .push((task_id, connection_id));
            Ok(())
        }

        async fn remove(
            &self,
            connection_id: String,
        ) -> Result<u64, PipelineTaskWebSocketConnectionError> {
            self.removals.lock().unwrap().push(connection_id);
            Ok(1)
        }
    }

    #[test]
    fn deserializes_a_connect_route_event() {
        let event: ApiGatewayWebSocketEvent = serde_json::from_str(
            r#"{
                "requestContext": {
                    "connectionId": "connection-123",
                    "routeKey": "$connect"
                },
                "isBase64Encoded": false
            }"#,
        )
        .unwrap();

        assert_eq!(
            event.request_context.connection_id,
            ConnectionId("connection-123".into())
        );
        assert_eq!(event.request_context.route_key, WebSocketRoute::Connect);
        assert_eq!(event.body, None);
    }

    #[test]
    fn deserializes_a_subscribe_request() {
        let request: SubscribeRequest = serde_json::from_str(r#"{"taskId": 42}"#).unwrap();

        assert_eq!(request.task_id, 42);
    }

    #[tokio::test]
    async fn dispatches_connect_subscribe_and_disconnect_events() {
        let connections = RecordingConnectionStore::default();
        let handler = TaskEventsHandler::new(connections.clone());
        for (route_key, body) in [
            ("$connect", None),
            ("subscribe", Some(r#"{"taskId":42}"#)),
            ("$disconnect", None),
        ] {
            let response = handler
                .handle(LambdaEvent::new(
                    ApiGatewayWebSocketEvent {
                        request_context: WebSocketRequestContext {
                            connection_id: ConnectionId("connection-123".into()),
                            route_key: serde_json::from_str(&format!("\"{route_key}\"")).unwrap(),
                        },
                        body: body.map(str::to_owned),
                        is_base64_encoded: false,
                    },
                    Default::default(),
                ))
                .await
                .unwrap();

            assert_eq!(response, success_response());
        }

        assert_eq!(
            connections.subscriptions.lock().unwrap().as_slice(),
            [(42, "connection-123".into())]
        );
        assert_eq!(
            connections.removals.lock().unwrap().as_slice(),
            ["connection-123"]
        );
    }

    #[tokio::test]
    async fn rejects_a_subscribe_event_without_a_body() {
        let error = TaskEventsHandler::new(RecordingConnectionStore::default())
            .handle(LambdaEvent::new(
                ApiGatewayWebSocketEvent {
                    request_context: WebSocketRequestContext {
                        connection_id: ConnectionId("connection-123".into()),
                        route_key: WebSocketRoute::Subscribe,
                    },
                    body: None,
                    is_base64_encoded: false,
                },
                Default::default(),
            ))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "subscribe requests require a body");
    }
}
