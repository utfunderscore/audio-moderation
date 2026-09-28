# Evaluation reads and task-event tickets

This Lambda serves `GetEvaluation` and `CreateTaskEventsTicket` for evaluations
linked to uploaded reviews. The client supplies its `review_v1` bearer token;
authorization resolves the review through the evaluation's review-job link.

Evaluation creation belongs to `SubmitReview` in `submit-audio-lambda`. The
upload notification handled by `confirm-upload-lambda` dispatches the workflow.
This Lambda cannot create or dispatch evaluations.

For deployed coverage of the upload-to-dispatch boundary, run the
`review-confirmation` suite through the root `deployment-integration.sh` runner.
See `docs/deployment-integration.md` for prerequisites and approval rules.
