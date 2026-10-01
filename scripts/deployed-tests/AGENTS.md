## Deployed test workflow

- Read `README.md` here before remote tests. Use
  `AWS_PROFILE=admin ./scripts/test-deployed.sh preflight <suite>`, then
  `test <suite>`, from the root. Obtain authorization for the selected remote test.
- Tests must never apply infrastructure or publish images. Source only the
  selected suite; keep fixture setup and cleanup with its owner.
- Review tests need a fresh real `TURNSTILE_TEST_TOKEN`. Never print tokens,
  database URLs, or decrypted secrets.
- Callback/transcription/moderation isolated suites remain unsupported. Do not
  bypass this with fake task tokens or alter model contracts to create fixtures.
- Review-confirmation verifies dispatch, not workflow completion. Retain
  fixtures while asynchronous work may still use them; never do broad cleanup.
- Run Cargo from `backend/` and select only the named ignored deployed test.
