# ── Portfolio Metrics Lambda ─────────────────────────────────────────────────
# Triggered by DynamoDB Streams on the sessions table.
# Reads portfolio_json from each updated real-user session (user_id > 0),
# computes CashBalance and TotalValue, and publishes them to CloudWatch.

data "archive_file" "portfolio_metrics" {
  type        = "zip"
  output_path = "${path.module}/portfolio_metrics_lambda.zip"

  source {
    filename = "lambda_function.py"
    content  = <<-PYTHON
      import json, os, boto3

      cloudwatch = boto3.client("cloudwatch")
      PROJECT_NAME = os.environ["PROJECT_NAME"]

      def lambda_handler(event, context):
          for record in event.get("Records", []):
              if record.get("eventName") not in ("INSERT", "MODIFY"):
                  continue

              new_image = record.get("dynamodb", {}).get("NewImage", {})
              if not new_image:
                  continue

              # DynamoDB Number type
              user_id = int(new_image.get("user_id", {}).get("N", "0"))
              # Skip test sessions (negative user IDs) and zero
              if user_id <= 0:
                  continue

              portfolio_json_str = new_image.get("portfolio_json", {}).get("S", "")
              if not portfolio_json_str:
                  continue

              try:
                  portfolio = json.loads(portfolio_json_str)
              except Exception:
                  continue

              # Rust Decimal serialises as a string, e.g. "123.45"
              cash = float(portfolio.get("balance", "0") or 0)

              positions_value = 0.0
              for pos in portfolio.get("open_positions", {}).values():
                  price = float(pos.get("current_price", "0") or 0)
                  qty   = float(pos.get("quantity", "0") or 0)
                  positions_value += price * qty

              total_value = cash + positions_value

              cloudwatch.put_metric_data(
                  Namespace="Falke/Portfolio",
                  MetricData=[
                      {
                          "MetricName": "CashBalance",
                          "Dimensions": [{"Name": "Project", "Value": PROJECT_NAME}],
                          "Value": cash,
                          "Unit": "None",
                      },
                      {
                          "MetricName": "TotalValue",
                          "Dimensions": [{"Name": "Project", "Value": PROJECT_NAME}],
                          "Value": total_value,
                          "Unit": "None",
                      },
                  ],
              )
    PYTHON
  }
}

# IAM role for the Lambda
resource "aws_iam_role" "portfolio_metrics_lambda" {
  name = "${local.name_prefix}-portfolio-metrics-lambda"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "lambda.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "portfolio_metrics_basic" {
  role       = aws_iam_role.portfolio_metrics_lambda.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "portfolio_metrics_lambda" {
  name = "portfolio-metrics-lambda-policy"
  role = aws_iam_role.portfolio_metrics_lambda.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "dynamodb:GetRecords",
          "dynamodb:GetShardIterator",
          "dynamodb:DescribeStream",
        ]
        Resource = aws_dynamodb_table.sessions.stream_arn
      },
      {
        Effect   = "Allow"
        Action   = ["dynamodb:ListStreams"]
        Resource = "*"
      },
      {
        Effect   = "Allow"
        Action   = ["cloudwatch:PutMetricData"]
        Resource = "*"
      },
    ]
  })
}

resource "aws_lambda_function" "portfolio_metrics" {
  function_name    = "${local.name_prefix}-portfolio-metrics"
  filename         = data.archive_file.portfolio_metrics.output_path
  source_code_hash = data.archive_file.portfolio_metrics.output_base64sha256
  handler          = "lambda_function.lambda_handler"
  runtime          = "python3.12"
  role             = aws_iam_role.portfolio_metrics_lambda.arn
  timeout          = 30

  environment {
    variables = {
      PROJECT_NAME = local.name_prefix
    }
  }
}

resource "aws_lambda_event_source_mapping" "sessions_stream" {
  event_source_arn  = aws_dynamodb_table.sessions.stream_arn
  function_name     = aws_lambda_function.portfolio_metrics.arn
  starting_position = "LATEST"
  batch_size        = 10
}
