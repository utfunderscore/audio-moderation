# Task-events Lambda

## Deployed integration test

`tests/deployed.rs` is ignored by default. Run it only through the root runner:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight task-events
AWS_PROFILE=admin ./deployment-integration.sh test task-events
```

The runner resolves Terraform's `pipeline_task_events_websocket_endpoint` into
`AUDIO_MODERATION_TASK_EVENTS_ENDPOINT`, validates its `wss://` scheme, and
loads the tenant and database configuration with the local `admin` AWS profile.
The test creates an isolated pipeline task, verifies durable replay and live
fanout, checks disconnect cleanup, then verifies ordered replay after reconnect.
It uses one task per socket and accepts text or binary UTF-8 event-name frames.

Task-event delivery is at-least-once. The test tolerates duplicates where replay
and live fanout meet, but requires the durable history's event order when no
producer is concurrent. Successful fixtures are deleted; a failed fixture is
retained with its pipeline task ID printed for diagnosis. The test never prints
database credentials or decrypted parameter values.
