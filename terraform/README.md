# Terraform maintenance map

Use this guide to locate infrastructure configuration when modifying the backend.
All `.tf` files form **one root module**, organized by owning component rather
than AWS resource type. A component's variables, resources, and outputs usually
live together; `variables.tf` and `outputs.tf` contain only shared configuration.

## Find files by the change you need

| Change or problem | Start here | Also inspect |
| --- | --- | --- |
| Lambda memory, timeout, environment, image, logs, or execution permissions | The owning Lambda file below | `task-events.tf` for shared event-publishing permissions |
| Review submission or presigned upload permissions | `submit-audio.tf` | `uploads.tf`, `turnstile.tf` |
| Upload does not trigger processing, or notification filters need changing | `confirm-upload.tf` | `uploads.tf`, `evaluation-workflow.tf` |
| Workflow steps, payloads, retries, timeouts, or failure handling | `evaluation-workflow.tf` | `audio-processing.tf`, caller files, `task-callback.tf` |
| Audio input/output bucket permissions | `audio-processing.tf` | `uploads.tf`, `artifacts.tf` |
| Evaluation reads or task-event ticket routing | `start-evaluation.tf` | `public-api.tf`, `task-events.tf` |
| Model endpoint configuration or proxy-token access | `transcription-caller.tf`, `moderation-caller.tf` | `variables.tf`, `modal-oidc.tf` |
| Callback route authentication or workflow-resumption permissions | `task-callback.tf` | `modal-oidc.tf`, `evaluation-workflow.tf` |
| WebSocket routing, endpoints, or lifecycle-event publishing permissions | `task-events.tf` | `start-evaluation.tf` and the publishing Lambda files |
| HTTP routes or integrations | The serving Lambda file | `public-api.tf` for API-wide settings |
| Browser API/upload CORS | `variables.tf` (`browser_allowed_origins`) | `public-api.tf`, `uploads.tf` |
| API domains, DNS, or certificates | `cloudflare.tf` | `public-api.tf` for HTTP, `task-events.tf` for WebSocket |
| Turnstile widget, frontend hostnames, or secret wiring | `turnstile.tf` | `submit-audio.tf`, `cloudflare.tf` |
| Modal workspace trust or artifact/callback access | `modal-oidc.tf` | `artifacts.tf`, `task-callback.tf` |
| S3 retention, encryption, or public-access settings | `uploads.tf`, `artifacts.tf` | Bucket consumers' IAM policies |
| Resource naming, tags, or application grouping | `main.tf` | `variables.tf`, `resource-group.tf` |
| Provider authentication, constraints, or dependency locks | `providers.tf`, `versions.tf` | `.terraform.lock.hcl` |
| Resource-address rename | `moved.tf` | The resource's owning file |
| Legacy callback-test infrastructure | `task-callback-test.tf` | `task-callback.tf` |

## Lambda-owned files

Each file owns its Lambda's image-tag input, ECR repository and pull/lifecycle
policies, execution role, component-specific IAM policies, log group, function
configuration, and component outputs. Triggers and HTTP integrations stay with
the Lambda that handles them.

| File | Component-specific configuration |
| --- | --- |
| `submit-audio.tf` | Review/upload RPC Lambda; S3 upload permissions; database and Turnstile-secret access; Turnstile hostname/action environment; HTTP `POST /{proxy+}` integration. |
| `confirm-upload.tf` | Upload-notification Lambda; source-object access; workflow-start permissions and environment; S3 invoke permission and `ObjectCreated` notification filtered to `reviews/…/source`. |
| `audio-processing.tf` | Audio conversion and task-finalization worker; input/output object permissions, including `audio_source_bucket_arns`; database and task-event environment. Workflow invocation and terminal EventBridge wiring live in `evaluation-workflow.tf`. |
| `start-evaluation.tf` | Evaluation-read and task-event-ticket Lambda; database/tenant environment; explicit `GetEvaluation` and `CreateTaskEventsTicket` HTTP routes. Despite its historical name, this component does **not** start the workflow. |
| `transcription-caller.tf` | Asynchronous transcription-request Lambda; `transcription_endpoint_url`; database and Modal proxy-token parameter access; callback URL and task-event environment. |
| `moderation-caller.tf` | Asynchronous moderation-request Lambda; `modal_endpoint_url` base URL; database and Modal proxy-token parameter access; callback URL and task-event environment. |
| `task-callback.tf` | External-result callback Lambda; Step Functions task-success/task-failure permissions; database access; `POST /callbacks/external-task` integration with `AWS_IAM` authentication; callback URL output. |
| `task-events.tf` | Task-event Lambda **and** WebSocket API; management/browser endpoint locals; `$connect`, `subscribe`, and `$disconnect` routes; stage, custom domain, API mapping, proxied DNS, and shared `task_event_emission` IAM policies for lifecycle Lambdas. |

## Shared component files

| File | Owns |
| --- | --- |
| `uploads.tf` | Upload bucket and random name suffix, AES256 encryption, public-access blocking, browser CORS, upload expiration, incomplete-multipart cleanup, and bucket-name output. The notification belongs to `confirm-upload.tf`, not this file. |
| `artifacts.tf` | Generated-artifact bucket and random name suffix, AES256 encryption, public-access blocking, artifact expiration, incomplete-multipart cleanup, and bucket-name output. |
| `evaluation-workflow.tf` | Standard Step Functions definition, execution role and invoke/logging permissions, workflow log group, and state-machine ARN output. Also owns terminal-status EventBridge reconciliation, its Lambda target, and invoke permission. |
| `public-api.tf` | Shared HTTP API, CORS, default stage/throttling, endpoint selection/output, optional regional custom domain, API mapping, and proxied DNS. Individual routes/integrations belong to their Lambda files. |
| `cloudflare.tf` | Cloudflare zone/account lookup, `enable_cloudflare_proxy`, shared domain map, ACM certificates, DNS-only certificate-validation records, and certificate validation for both APIs. The zone lookup also supplies the Turnstile account when proxying is disabled. |
| `turnstile.tf` | Managed review-submission widget, exact frontend hostname allowlist, Terraform-managed SSM secret parameter, and public sitekey output. Consumer permissions/environment belong to `submit-audio.tf`. |
| `modal-oidc.tf` | Lookup of the existing Modal OIDC provider, workspace-scoped role trust, artifact read permissions under `evaluations/`, Lambda/callback-route invocation permissions, and role ARN output. It does not create the OIDC provider or deploy model services. |
| `task-callback-test.tf` | Legacy callback-test state machine, its role/policy, and optional ARN output, all gated by `enable_test_resources`. This helper alone does not provide the persisted callback-attempt fixtures needed by current callback handling. |
| `resource-group.tf` | Application resource group selected by project/environment tags, plus group name/ARN outputs. |

## Root configuration and supporting files

| File | Owns |
| --- | --- |
| `main.tf` | Current AWS account identity, shared `<project>-<environment>` name prefix, and `Project`, `Environment`, `ManagedBy` tags. No component resources. |
| `variables.tf` | Shared region/project/environment, database and Modal credential parameter names, demo tenant, and browser-origin inputs. Component-specific inputs remain in their owning files. |
| `outputs.tf` | Shared `aws_region`, `database_parameter_name`, and `tenant_id` outputs only. Find component outputs at the bottom of their owning files. |
| `providers.tf` | AWS region/default-tag wiring and Cloudflare provider configuration. Cloudflare credentials come from the environment, not Terraform inputs. |
| `versions.tf` | Terraform minimum version and AWS, Cloudflare, and Random provider constraints. |
| `.terraform.lock.hcl` | Locked provider selections and checksums; keep aligned with deliberate provider changes. |
| `moved.tf` | Historical resource-address mappings from `upload_complete` to `confirm_upload`. File-only rearrangements do not need new mappings. |

`.terraform/`, `terraform.tfstate`, and state backups are generated local data,
not implementation files. Do not edit them to change infrastructure behavior.

## Cross-file relationships to preserve

### Upload dispatch and workflow completion

`submit-audio.tf` configures review submission; application code creates the
idempotent pipeline task. `confirm-upload.tf` owns the upload notification and
starts `aws_sfn_state_machine.audio_processing` from `evaluation-workflow.tf`.

The workflow invokes conversion, transcription, moderation, and finalization.
Step inputs/outputs and callback task tokens are defined in its state-machine
JSON, while the worker/caller resources live in their own files. When changing
payloads or callback behavior, inspect both the workflow definition and the
corresponding backend/model contracts.

Terminal EventBridge events in `evaluation-workflow.tf` invoke the conversion/
finalization worker too, including for stopped executions. Changes to completion
handling must account for both normal workflow finalization and reconciliation.

### API domains and browser access

`cloudflare.tf` supplies certificates for both APIs. Each API file owns its
custom domain, mapping, proxied DNS record, and endpoint selection. Changes to
domain behavior may therefore span all three files.

`browser_allowed_origins` in `variables.tf` feeds both HTTP API and upload-bucket
CORS. Turnstile's frontend hostname list is a separate input in `turnstile.tf`;
API hostnames and browser origins are not substitutes for it.

### External models, callbacks, and task events

The caller files configure endpoint URLs and credential access; `modal-oidc.tf`
controls the external workspace's AWS access; `task-callback.tf` controls callback
authentication and workflow resumption. An endpoint/configuration change is not
itself a change to the shared JSON request/callback contract.

`task-events.tf` owns publishing permissions for `audio_processing`,
`confirm_upload`, `task_callback`, `transcription_caller`, and `moderation_caller`.
Their function environments reference its management endpoint. When adding a
new publisher, update both the shared policy map and the publisher's wiring.
Tickets are served through `start-evaluation.tf`; event frames contain names,
not transcripts or scores, and replay/live delivery is at-least-once.

## Related implementation outside this directory

- `../backend/crates/`: Lambda handlers and supporting persistence/event code.
  Match the component above to its `*-lambda` crate; `database` owns persistence
  and `task-event-emitter` owns shared lifecycle event emission.
- `../backend/Dockerfile.lambda`: container build/runtime stages. A Lambda
  image/runtime change may also require changes here.
- `../proto/`: public RPC definitions; inspect when adding or changing HTTP RPCs.
- `../migrations/`: database schema; no database/schema provisioning lives here.
- `../models/`: separately deployed Python services sharing request/callback
  contracts with Rust callers.
- `../ui/src/api/`: browser RPC, upload, Turnstile, and task-event transport.
- `../deployment-integration.sh`: consumer of Terraform input/output names and
  explicit ECR bootstrap resource addresses. Check references when renaming them.
- `../ui/scripts/sync-deployment-env.sh`: consumer of public API, WebSocket, and
  Turnstile outputs. Check it when changing output names or endpoint semantics.

## Placement rules for future changes

- Put component-specific variables at the top of the owning file and outputs at
  the bottom. Do not turn the shared input/output files into global inventories.
- Keep Lambda execution policies, routes, and triggers with that Lambda, except
  genuinely shared wiring explicitly owned by the workflow or task-event files.
- Extend shared component files for shared resources rather than duplicating
  bucket, API, certificate, or account configuration in Lambda files.
- Preserve resource addresses and input/output names unless changing those
  interfaces deliberately. Renames can affect `moved.tf` and external consumers;
  moving an unchanged block between files does not change its Terraform address.
