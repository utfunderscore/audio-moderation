## Deployment workflow

- Read `README.md` here before remote deployment operations. Use
  `AWS_PROFILE=admin ./scripts/deploy.sh preflight`, then `deploy`, from the root.
- AWS deployment requires explicit user approval. Modal modifications require
  explicit approval immediately before the command; this runner does not deploy Modal.
- Preserve the eight-image immutable-tag sequence and the authoritative full apply.
  Internal image/Terraform modules are not independently runnable entry points.
- Terraform state is local; never deploy concurrently from separate worktrees.
- Preflight checks parameter metadata only. Never print decrypted secrets or
  commit Terraform state. Migrations are a separate prerequisite, not runner work.
