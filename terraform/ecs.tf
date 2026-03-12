# ECR Repository
resource "aws_ecr_repository" "app" {
  name                 = local.name_prefix
  image_tag_mutability = "MUTABLE"
  force_delete         = true

  image_scanning_configuration {
    scan_on_push = true
  }

  tags = { Name = "${local.name_prefix}-ecr" }
}

# ECS Cluster
resource "aws_ecs_cluster" "main" {
  name = local.name_prefix

  setting {
    name  = "containerInsights"
    value = "enabled"
  }

  tags = { Name = "${local.name_prefix}-cluster" }
}

# ECS Task Execution Role (for pulling images, reading secrets)
resource "aws_iam_role" "ecs_execution" {
  name = "${local.name_prefix}-ecs-execution"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action = "sts:AssumeRole"
      Effect = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
    }]
  })
}

resource "aws_iam_role_policy_attachment" "ecs_execution" {
  role       = aws_iam_role.ecs_execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_iam_role_policy" "ecs_execution_secrets" {
  name = "${local.name_prefix}-secrets"
  role = aws_iam_role.ecs_execution.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Action = [
        "secretsmanager:GetSecretValue"
      ]
      Resource = [
        aws_secretsmanager_secret.telegram_token.arn,
        aws_secretsmanager_secret.allowed_phones.arn,
        aws_secretsmanager_secret.wallet_key.arn,
      ]
    }]
  })
}

# ECS Task Role (for DynamoDB access from within the container)
resource "aws_iam_role" "ecs_task" {
  name = "${local.name_prefix}-ecs-task"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action = "sts:AssumeRole"
      Effect = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
    }]
  })
}

resource "aws_iam_role_policy" "ecs_task_dynamo" {
  name = "${local.name_prefix}-dynamo"
  role = aws_iam_role.ecs_task.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Action = [
        "dynamodb:GetItem",
        "dynamodb:PutItem",
        "dynamodb:Query",
        "dynamodb:UpdateItem",
        "dynamodb:DeleteItem",
        "dynamodb:Scan",
      ]
      Resource = [
        aws_dynamodb_table.users.arn,
        aws_dynamodb_table.trades.arn,
        "${aws_dynamodb_table.trades.arn}/index/*",
      ]
    }]
  })
}

# CloudWatch Log Group
resource "aws_cloudwatch_log_group" "app" {
  name              = "/ecs/${local.name_prefix}"
  retention_in_days = 30

  tags = { Name = "${local.name_prefix}-logs" }
}

# ECS Task Definition
resource "aws_ecs_task_definition" "app" {
  family                   = local.name_prefix
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = var.ecs_cpu
  memory                   = var.ecs_memory
  execution_role_arn       = aws_iam_role.ecs_execution.arn
  task_role_arn            = aws_iam_role.ecs_task.arn

  container_definitions = jsonencode([{
    name  = local.name_prefix
    image = "${aws_ecr_repository.app.repository_url}:latest"

    essential = true

    environment = [
      { name = "TRADING_MODE", value = var.trading_mode },
      { name = "PAPER_BALANCE", value = var.paper_balance },
      { name = "AWS_REGION", value = var.aws_region },
      { name = "DYNAMO_TABLE_PREFIX", value = "${var.project_name}-${var.environment}" },
      { name = "RUST_LOG", value = "falke=info" },
      { name = "GAMMA_API_URL", value = "https://gamma-api.polymarket.com" },
      { name = "CLOB_API_URL", value = "https://clob.polymarket.com" },
    ]

    secrets = [
      {
        name      = "TELEGRAM_BOT_TOKEN"
        valueFrom = aws_secretsmanager_secret.telegram_token.arn
      },
      {
        name      = "ALLOWED_PHONES"
        valueFrom = aws_secretsmanager_secret.allowed_phones.arn
      },
      {
        name      = "WALLET_PRIVATE_KEY"
        valueFrom = aws_secretsmanager_secret.wallet_key.arn
      },
    ]

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        "awslogs-group"         = aws_cloudwatch_log_group.app.name
        "awslogs-region"        = var.aws_region
        "awslogs-stream-prefix" = "ecs"
      }
    }
  }])

  tags = { Name = "${local.name_prefix}-task" }
}

# ECS Service
resource "aws_ecs_service" "app" {
  name            = local.name_prefix
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.app.arn
  desired_count   = 1
  launch_type     = "FARGATE"

  network_configuration {
    subnets          = aws_subnet.private[*].id
    security_groups  = [aws_security_group.ecs.id]
    assign_public_ip = false
  }

  # Allow task to be updated without destroying
  force_new_deployment = true

  tags = { Name = "${local.name_prefix}-service" }
}
