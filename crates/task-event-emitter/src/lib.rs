//! WebSocket delivery of pipeline task event names.

use std::io::Error as IoError;

use aws_sdk_apigatewaymanagement::{
    Client as ApiGatewayManagementClient, config::Builder as ApiGatewayManagementConfigBuilder,
    error::ProvideErrorMetadata, primitives::Blob,
};
use aws_types::SdkConfig;
use database::{
    NewPipelineTaskEvent, PipelineTaskEventStore, PipelineTaskWebSocketConnectionStore,
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Counts produced by a task-event fanout attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskEventEmission {
    pub delivered: usize,
    pub removed_stale_connections: usize,
}

/// Emits task event names to WebSocket connections subscribed in PostgreSQL.
#[derive(Clone)]
pub struct TaskEventEmitter {
    events: PipelineTaskEventStore,
    connections: PipelineTaskWebSocketConnectionStore,
    client: ApiGatewayManagementClient,
}

impl TaskEventEmitter {
    /// Creates an emitter for an API Gateway WebSocket management endpoint.
    ///
    /// `management_endpoint` must be an HTTPS endpoint for the deployed API
    /// and stage, not its client-facing WSS URL.
    pub fn new(
        events: PipelineTaskEventStore,
        connections: PipelineTaskWebSocketConnectionStore,
        sdk_config: &SdkConfig,
        management_endpoint: impl Into<String>,
    ) -> Self {
        let config = ApiGatewayManagementConfigBuilder::from(sdk_config)
            .endpoint_url(management_endpoint)
            .build();
        Self {
            events,
            connections,
            client: ApiGatewayManagementClient::from_conf(config),
        }
    }

    /// Sends an event name to every subscriber for `task_id`.
    ///
    /// A closed connection is removed before fanout continues. Other delivery
    /// failures are returned to the caller so it can choose whether to retry.
    pub async fn emit(&self, task_id: i32, event_name: &str) -> Result<TaskEventEmission, Error> {
        self.events
            .create_or_get(NewPipelineTaskEvent {
                task_id,
                event_name,
            })
            .await?;
        let connections = self.connections.list(task_id).await?;
        let mut emission = TaskEventEmission::default();

        for connection in connections {
            let send_result = self
                .client
                .post_to_connection()
                .connection_id(&connection.connection_id)
                .data(Blob::new(event_name.as_bytes()))
                .send()
                .await;

            match send_result {
                Ok(_) => emission.delivered += 1,
                Err(error)
                    if error
                        .as_service_error()
                        .and_then(ProvideErrorMetadata::code)
                        == Some("GoneException") =>
                {
                    self.connections.remove(&connection.connection_id).await?;
                    emission.removed_stale_connections += 1;
                }
                Err(error) => {
                    return Err(IoError::other(format!(
                        "failed to post task event to WebSocket connection: {error}"
                    ))
                    .into());
                }
            }
        }

        Ok(emission)
    }
}
