use std::future::Future;

use database::{
    NewPipelineTaskWebSocketConnection, PipelineTaskEventTicketError, PipelineTaskEventTicketStore,
    PipelineTaskWebSocketConnectionError, PipelineTaskWebSocketConnectionStore,
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
    pub ticket: String,
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
pub struct TaskEventsHandler<S, T> {
    connections: S,
    tickets: T,
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

pub trait TaskEventTicketStore: Send + Sync {
    fn consume(
        &self,
        ticket: String,
    ) -> impl Future<Output = Result<i32, PipelineTaskEventTicketError>> + Send;
}

impl TaskEventTicketStore for PipelineTaskEventTicketStore {
    async fn consume(&self, ticket: String) -> Result<i32, PipelineTaskEventTicketError> {
        PipelineTaskEventTicketStore::consume(self, &ticket)
            .await
            .map(|ticket| ticket.task_id)
    }
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

impl<S, T> TaskEventsHandler<S, T> {
    pub fn new(connections: S, tickets: T) -> Self {
        Self {
            connections,
            tickets,
            events: None,
        }
    }

    pub fn with_event_emitter(mut self, events: TaskEventEmitter) -> Self {
        self.events = Some(events);
        self
    }
}

impl<S: TaskConnectionStore, T: TaskEventTicketStore> TaskEventsHandler<S, T> {
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
        let task_id = match self.tickets.consume(event.request.ticket).await {
            Ok(task_id) => task_id,
            Err(PipelineTaskEventTicketError::Invalid) => return Ok(unauthorized_response()),
            Err(error) => return Err(error.into()),
        };
        self.connections
            .subscribe(task_id, event.connection_id.0.clone())
            .await?;
        if let Some(events) = &self.events {
            let replay = events.replay(task_id, &event.connection_id.0).await?;
            info!(
                connectionId = %event.connection_id.0,
                taskId = task_id,
                replayedEvents = replay.delivered,
                removedStaleConnections = replay.removed_stale_connections,
                "replayed WebSocket task events"
            );
        }
        info!(
            connectionId = %event.connection_id.0,
            taskId = task_id,
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

fn unauthorized_response() -> WebSocketResponse {
    WebSocketResponse {
        status_code: 401,
        body: Some("invalid or expired task-events ticket".into()),
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

    #[derive(Clone, Default)]
    struct RecordingTicketStore {
        consumed: Arc<Mutex<Vec<String>>>,
    }

    impl TaskEventTicketStore for RecordingTicketStore {
        async fn consume(&self, ticket: String) -> Result<i32, PipelineTaskEventTicketError> {
            self.consumed.lock().unwrap().push(ticket.clone());
            if ticket == "ticket-secret" {
                Ok(42)
            } else {
                Err(PipelineTaskEventTicketError::Invalid)
            }
        }
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
        let request: SubscribeRequest =
            serde_json::from_str(r#"{"ticket":"ticket-secret"}"#).unwrap();

        assert_eq!(request.ticket, "ticket-secret");
    }

    #[tokio::test]
    async fn dispatches_connect_subscribe_and_disconnect_events() {
        let connections = RecordingConnectionStore::default();
        let tickets = RecordingTicketStore::default();
        let handler = TaskEventsHandler::new(connections.clone(), tickets.clone());
        for (route_key, body) in [
            ("$connect", None),
            ("subscribe", Some(r#"{"ticket":"ticket-secret"}"#)),
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
        assert_eq!(
            tickets.consumed.lock().unwrap().as_slice(),
            ["ticket-secret"]
        );
    }

    #[tokio::test]
    async fn rejects_a_subscribe_event_without_a_body() {
        let error = TaskEventsHandler::new(
            RecordingConnectionStore::default(),
            RecordingTicketStore::default(),
        )
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

    #[tokio::test]
    async fn rejects_an_invalid_ticket_without_subscribing() {
        let connections = RecordingConnectionStore::default();
        let response = TaskEventsHandler::new(connections.clone(), RecordingTicketStore::default())
            .subscribe(SubscribeEvent {
                connection_id: ConnectionId("connection-123".into()),
                request: SubscribeRequest {
                    ticket: "invalid".into(),
                },
            })
            .await
            .unwrap();

        assert_eq!(response, unauthorized_response());
        assert!(connections.subscriptions.lock().unwrap().is_empty());
    }
}
