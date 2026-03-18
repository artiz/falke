# CloudWatch Dashboard
resource "aws_cloudwatch_dashboard" "main" {
  dashboard_name = local.name_prefix

  dashboard_body = jsonencode({
    widgets = [
      {
        type   = "metric"
        x      = 0
        y      = 0
        width  = 12
        height = 6
        properties = {
          metrics = [
            ["AWS/ECS", "CPUUtilization", "ServiceName", local.name_prefix, "ClusterName", local.name_prefix],
            ["AWS/ECS", "MemoryUtilization", "ServiceName", local.name_prefix, "ClusterName", local.name_prefix],
          ]
          period = 300
          stat   = "Average"
          region = var.aws_region
          title  = "ECS CPU & Memory"
        }
      },
      {
        type   = "log"
        x      = 0
        y      = 6
        width  = 24
        height = 6
        properties = {
          query          = "SOURCE '/ecs/${local.name_prefix}' | fields @timestamp, @message | filter @message like /TAIL|RESOLVED|PAPER TRADE|LIVE ORDER|P&L|ERROR/ | sort @timestamp desc | limit 50"
          logGroupNames  = ["/ecs/${local.name_prefix}"]
          region         = var.aws_region
          stacked        = false
          view           = "table"
          title          = "Trading Signals & Trades"
        }
      },
      {
        type   = "metric"
        x      = 12
        y      = 0
        width  = 12
        height = 6
        properties = {
          metrics = [
            ["AWS/DynamoDB", "ConsumedReadCapacityUnits", "TableName", "${local.name_prefix}-trades"],
            ["AWS/DynamoDB", "ConsumedWriteCapacityUnits", "TableName", "${local.name_prefix}-trades"],
          ]
          period = 300
          stat   = "Sum"
          region = var.aws_region
          title  = "DynamoDB Activity"
        }
      },
    ]
  })
}
