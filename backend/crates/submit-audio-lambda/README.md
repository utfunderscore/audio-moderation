# SubmitReview Turnstile

At startup this Lambda reads `TURNSTILE_SECRET_KEY_PARAMETER` through SSM SecureString
and requires `TURNSTILE_ALLOWED_HOSTNAMES` (comma-separated exact frontend
hostnames, without schemes or ports). `TURNSTILE_EXPECTED_ACTION` defaults to
`submit_review` and must not be empty. New reviews require a nonempty
`turnstileToken` of at most 2048 bytes; Siteverify must return success, an
allowed hostname, and the expected action. HTTP/response errors fail closed.
Only an authenticated idempotent replay can skip Siteverify for an already
created review; it requires the bearer token but does not require a
`turnstileToken`. New verification attempts use a backend-generated random v4
UUID for Siteverify's idempotency key, never the client submission UUID.

The ignored deployed review tests read `TURNSTILE_TEST_TOKEN`. Real Turnstile
tokens are single-use, so obtain a fresh token for each new submission. The
Terraform-managed widget has a real secret: dummy tokens will not validate
against it. Local verification tests mock Siteverify.
