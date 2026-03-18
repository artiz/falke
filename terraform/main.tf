terraform {
  required_version = ">= 1.5"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }

  # Key is passed via -backend-config at init time, e.g.:
  #   terraform init -backend-config="key=state/prod/terraform.tfstate"
  # The deploy.sh script handles this automatically via tf_init().
  backend "s3" {
    bucket = "falke-tf-state"
    region = "eu-west-2"
  }
}

provider "aws" {
  region = var.aws_region

  default_tags {
    tags = {
      Project     = var.project_name
      Environment = var.environment
      ManagedBy   = "terraform"
    }
  }
}

locals {
  name_prefix = "${var.project_name}-${var.environment}"
}
