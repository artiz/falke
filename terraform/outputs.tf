output "ecr_repository_url" {
  description = "ECR repository URL for Docker push"
  value       = aws_ecr_repository.app.repository_url
}

output "ecs_cluster_name" {
  description = "ECS cluster name"
  value       = aws_ecs_cluster.main.name
}

output "ecs_service_name" {
  description = "ECS service name"
  value       = aws_ecs_service.app.name
}

output "users_table_name" {
  description = "DynamoDB users table name"
  value       = aws_dynamodb_table.users.name
}

output "trades_table_name" {
  description = "DynamoDB trades table name"
  value       = aws_dynamodb_table.trades.name
}

output "log_group_name" {
  description = "CloudWatch log group"
  value       = aws_cloudwatch_log_group.app.name
}

output "deploy_commands" {
  description = "Commands to deploy a new version"
  value       = <<-EOT
    # Build and push Docker image:
    aws ecr get-login-password --region ${var.aws_region} | docker login --username AWS --password-stdin ${aws_ecr_repository.app.repository_url}
    docker build -t ${local.name_prefix} .
    docker tag ${local.name_prefix}:latest ${aws_ecr_repository.app.repository_url}:latest
    docker push ${aws_ecr_repository.app.repository_url}:latest

    # Force new deployment:
    aws ecs update-service --cluster ${local.name_prefix} --service ${local.name_prefix} --force-new-deployment --region ${var.aws_region}

    # View logs:
    aws logs tail /ecs/${local.name_prefix} --follow --region ${var.aws_region}
  EOT
}
