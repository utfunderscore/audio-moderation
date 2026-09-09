# Start evaluation Lambda

## Deployed integration tests

`tests/deployed.rs` is ignored by default. It calls the deployed Connect endpoint, reads the same PostgreSQL database used by the Lambda, and describes the resulting Step Functions executions. It requires:

- a separately deployed `start-evaluation` Lambda and route; `deploy-upload-flow.sh` does **not** deploy this Lambda;
- applied Terraform for that deployment;
- AWS credentials for the local `admin` profile. When configuration is resolved automatically, they must allow Terraform state access, `lambda:GetFunctionConfiguration`, and `ssm:GetParameter`/KMS decryption. Step Functions assertions also require `states:DescribeExecution`; `--audio-file` additionally requires `s3:PutObject` on the uploads bucket; and
- either `AUDIO_MODERATION_TEST_AUDIO_S3_URIS` set to a JSON array containing at least two pre-uploaded, valid audio object URIs readable by the deployed workflow, with the first two distinct, for example `["s3://bucket/one.wav","s3://bucket/two.wav"]`; or a regular, readable, nonempty local audio file passed with `--audio-file`.

The runner automatically resolves omitted `AUDIO_MODERATION_API_ENDPOINT` from
the Terraform `api_endpoint` output; `AWS_REGION` from the region component of
the `audio_processing_state_machine_arn` output; and
`AUDIO_MODERATION_TENANT_ID` and `DATABASE_URL` from the deployed Lambda. For
the database URL, it reads the Lambda's `DATABASE_URL_PARAMETER` and decrypts
that SSM parameter. `terraform`, `aws`, and `jq` are required when this
discovery is needed and whenever `--audio-file` provisions fixtures. Set any
of `AUDIO_MODERATION_API_ENDPOINT`, `AWS_REGION`,
`AUDIO_MODERATION_TENANT_ID`, or `DATABASE_URL` explicitly to override its
automatic resolution.

Run all six tests with the repository-root runner:

```sh
./run-start-evaluation-tests.sh
# Or upload one local file to two distinct fixture keys for this run.
./run-start-evaluation-tests.sh --audio-file ./sample_071.mp3
```

The runner works from any caller directory, validates configuration without
printing secret values, sets `AWS_PROFILE=admin`, compiles the deployed test
target before running it, and does not deploy infrastructure. With
`--audio-file`, it resolves the path before changing to the repository root,
uses `/proc/sys/kernel/random/uuid` to create a unique run ID, gets
`uploads_bucket_name` from Terraform, uploads the file twice to
`reviews/<run-id>/first/source` and `reviews/<run-id>/second/source`, and sets
`AUDIO_MODERATION_TEST_AUDIO_S3_URIS` to those two URIs. Do not set both the
flag and the environment variable. Use `./run-start-evaluation-tests.sh --help`
for details.

The tests intentionally do not clean up their database rows or Step Functions executions so failures remain diagnosable. `--audio-file` also does not clean up its uploaded objects: workflows are asynchronous and the bucket lifecycle retains and eventually removes them. Running dispatch cases, including fixture uploads, uses real AWS resources and has AWS cost; provide only real audio objects, never placeholder object URIs.

Fixtures are seeded through `PipelineTaskStore`; read-only SQL verifies persisted state without accidentally creating missing records. Unique idempotency keys isolate each run. The suite checks ingress and dispatch, not downstream audio processing completion. Use a dedicated development environment with the current schema already applied; the tests do not run migrations.
