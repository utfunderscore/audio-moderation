# AWS deployment

From the repository root:

```sh
AWS_PROFILE=admin ./scripts/deploy.sh preflight [options]
AWS_PROFILE=admin ./scripts/deploy.sh deploy [options]
```

Use `--help` for configuration flags and defaults. `deploy` always runs its own
preflight. AWS deployment requires explicit user approval; `--auto-approve`
only suppresses Terraform prompts, not that approval requirement. Any Modal
modification requires explicit approval immediately before its command; this
runner does not modify Modal applications.

## Prerequisites

- The `admin` AWS profile, AWS CLI, Terraform, Cargo, Docker, Git, and jq.
- Current database migrations applied separately. Preflight does not run
  migrations or verify schema objects.
- Three existing SSM `SecureString` parameters: database URL, Modal proxy token
  ID, and Modal proxy token secret. Preflight checks types without decrypting.
- The account's `oidc.modal.com` IAM OIDC provider and correct Modal workspace.
- Compatible HTTPS transcription and moderation endpoints. Explicit flags take
  precedence, then Terraform outputs, then deployed Lambda configuration.
  `--modal-endpoint-url` is a base URL; moderation appends `/moderation/`.
  Preflight validates configuration without contacting model endpoints.
- Cloudflare credentials (`CLOUDFLARE_API_TOKEN`, or `CLOUDFLARE_API_KEY` and
  `CLOUDFLARE_EMAIL`) and an explicit `TURNSTILE_ALLOWED_HOSTNAMES` allowlist.
  Hostnames must be exact lowercase names without schemes, ports, paths,
  whitespace, or wildcards. Include development hostnames only when needed.

Terraform creates the Turnstile widget and its fourth SSM secret parameter.
Terraform state contains the widget secret: never commit state or print secrets.

## Deployment sequence and safety

The runner uses the `admin` profile regardless of the caller's default. Prefix
all AWS CLI, Terraform, and deployment commands with `AWS_PROFILE=admin`.

1. Validate prerequisites and select a git-SHA/timestamp tag (or `--image-tag`).
2. Initialize Terraform and apply refresh-only state moves.
3. Bootstrap the eight ECR repositories and their policies.
4. Check the tag is absent in every repository, then build/push all eight images.
5. Perform one authoritative full apply with that tag for every Lambda.
6. Wait for every Lambda to update and synchronize public UI environment values.

Never use `latest`, publish ad hoc images, or bypass this sequence with a direct
Terraform apply. A failed publication can leave a partially published tag;
retry with a new immutable tag. The sequence coordinates tags but is not an
atomic rollout or automatic rollback.

Terraform state is local in `terraform/`. Do not deploy concurrently from
separate worktrees: there is no shared state lock. UI synchronization writes only
public outputs to ignored `ui/.env.local` and `ui/.env.production.local` files.

## Implementation boundary

`config.sh` owns deployment-only settings, `preflight.sh` validates prerequisites,
`images.sh` owns the shared Lambda list and image publication, and `terraform.sh`
coordinates the deployment. These are internal modules, not independent public
commands. Deployment never loads deployed-test suites, fixture SQL, or decrypted
database URLs. For existing-deployment verification, see
[`../deployed-tests/README.md`](../deployed-tests/README.md).
