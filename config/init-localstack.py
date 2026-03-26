import boto3

print("Initializing LocalStack...")

dynamodb = boto3.client(
    "dynamodb",
    endpoint_url="http://localhost:4566",
    region_name="eu-west-1",
    aws_access_key_id="test",
    aws_secret_access_key="test",
)

# Users table
try:
    dynamodb.create_table(
        TableName="falke-dev-users",
        KeySchema=[{"AttributeName": "telegram_id", "KeyType": "HASH"}],
        AttributeDefinitions=[{"AttributeName": "telegram_id", "AttributeType": "N"}],
        BillingMode="PAY_PER_REQUEST",
    )
    print("Created table: falke-dev-users")
except dynamodb.exceptions.ResourceInUseException:
    print("Table falke-dev-users already exists")

# Trades table
try:
    dynamodb.create_table(
        TableName="falke-dev-trades",
        KeySchema=[{"AttributeName": "trade_id", "KeyType": "HASH"}],
        AttributeDefinitions=[
            {"AttributeName": "trade_id", "AttributeType": "S"},
            {"AttributeName": "user_id", "AttributeType": "N"},
        ],
        GlobalSecondaryIndexes=[
            {
                "IndexName": "user_id-index",
                "KeySchema": [{"AttributeName": "user_id", "KeyType": "HASH"}],
                "Projection": {"ProjectionType": "ALL"},
            }
        ],
        BillingMode="PAY_PER_REQUEST",
    )
    print("Created table: falke-dev-trades")
except dynamodb.exceptions.ResourceInUseException:
    print("Table falke-dev-trades already exists")

# Sessions table — stores serialized portfolios for session restore
try:
    dynamodb.create_table(
        TableName="falke-dev-sessions",
        KeySchema=[{"AttributeName": "user_id", "KeyType": "HASH"}],
        AttributeDefinitions=[{"AttributeName": "user_id", "AttributeType": "N"}],
        BillingMode="PAY_PER_REQUEST",
    )
    print("Created table: falke-dev-sessions")
except dynamodb.exceptions.ResourceInUseException:
    print("Table falke-dev-sessions already exists")

# Settings table — stores global bot settings (paused state, strategy params)
try:
    dynamodb.create_table(
        TableName="falke-dev-settings",
        KeySchema=[{"AttributeName": "settings_id", "KeyType": "HASH"}],
        AttributeDefinitions=[{"AttributeName": "settings_id", "AttributeType": "S"}],
        BillingMode="PAY_PER_REQUEST",
    )
    print("Created table: falke-dev-settings")
except dynamodb.exceptions.ResourceInUseException:
    print("Table falke-dev-settings already exists")

print("LocalStack initialization complete!")
