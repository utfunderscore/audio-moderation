# Evaluation query Lambda

This Lambda retains the `GetEvaluation` and `CreateTaskEventsTicket` Connect RPCs.
Both accept the short-lived review capability returned by `SubmitReview` after
verifying that the requested
evaluation is the one linked to that tenant's review job.
It no longer creates or dispatches evaluations: `SubmitReview` and its S3 upload
notification own that flow. The standalone `task-events` deployed suite covers
ticket and WebSocket delivery; `evaluation-e2e` lives with `submit-audio-lambda`.
