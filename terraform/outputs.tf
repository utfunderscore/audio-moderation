# Shared deployment outputs. Component-specific outputs live alongside their
# resources; their names remain part of the deployment runner's interface.
output "aws_region" {
  value = var.aws_region
}

output "database_parameter_name" {
  value = var.database_parameter_name
}

output "tenant_id" {
  value = var.tenant_id
}
