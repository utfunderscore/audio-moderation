resource "aws_resourcegroups_group" "application" {
  name        = local.name_prefix
  description = "Resources managed for the ${local.name_prefix} environment"

  resource_query {
    query = jsonencode({
      ResourceTypeFilters = ["AWS::AllSupported"]
      TagFilters = [
        {
          Key    = "Project"
          Values = [var.project_name]
        },
        {
          Key    = "Environment"
          Values = [var.environment]
        },
      ]
    })
  }
}

output "resource_group_name" {
  value = aws_resourcegroups_group.application.name
}

output "resource_group_arn" {
  value = aws_resourcegroups_group.application.arn
}
