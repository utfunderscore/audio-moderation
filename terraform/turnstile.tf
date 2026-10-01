variable "turnstile_secret_key_parameter_name" {
  description = "SecureString SSM parameter containing the Turnstile Siteverify secret (not the public sitekey)"
  type        = string
  default     = "/audio-moderation/dev/turnstile-secret-key"

  validation {
    condition     = can(regex("^/[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$", var.turnstile_secret_key_parameter_name))
    error_message = "turnstile_secret_key_parameter_name must be an absolute SSM parameter path such as /project/environment/turnstile-secret-key."
  }
}

variable "turnstile_allowed_hostnames" {
  description = "Comma-separated exact frontend hostnames accepted from Turnstile Siteverify (no schemes, ports, paths, or wildcards)"
  type        = string

  validation {
    condition = alltrue([
      for hostname in split(",", var.turnstile_allowed_hostnames) :
      can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$", hostname))
    ])
    error_message = "turnstile_allowed_hostnames must be a nonempty comma-separated list of lowercase exact hostnames without spaces, schemes, ports, or wildcards."
  }
}

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

output "turnstile_sitekey" {
  description = "Public Turnstile widget sitekey for the UI build"
  value       = cloudflare_turnstile_widget.submit_review.sitekey
}
