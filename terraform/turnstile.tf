resource "cloudflare_turnstile_widget" "submit_review" {
  account_id = data.cloudflare_zone.public[0].account.id
  name       = "${local.name_prefix}-submit-review"
  domains    = split(",", var.turnstile_allowed_hostnames)
  mode       = "managed"
}

resource "aws_ssm_parameter" "turnstile_secret" {
  name        = var.turnstile_secret_key_parameter_name
  description = "Cloudflare Turnstile Siteverify secret for SubmitReview"
  type        = "SecureString"
  value       = cloudflare_turnstile_widget.submit_review.secret
}
