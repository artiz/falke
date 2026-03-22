#!/bin/bash
# Falke AWS Deployment Script
# Manages Docker build/push to ECR and ECS Fargate deployments via Terraform.

set -e

# ── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_status()  { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
print_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }
print_error()   { echo -e "${RED}[ERROR]${NC} $1"; }

# ── Defaults ─────────────────────────────────────────────────────────────────
ENVIRONMENT="dev"
AWS_REGION="eu-west-2"
PROJECT_NAME="falke"
TERRAFORM_DIR="$(cd "$(dirname "$0")/../terraform" && pwd)"
IMAGE_TAG="${IMAGE_TAG:-latest}"

# ── Prerequisites ─────────────────────────────────────────────────────────────
check_prerequisites() {
    print_status "Checking prerequisites..."
    local missing=()
    command -v aws      &>/dev/null || missing+=("aws-cli")
    command -v terraform &>/dev/null || missing+=("terraform")
    command -v docker   &>/dev/null || missing+=("docker")
    command -v jq       &>/dev/null || missing+=("jq")

    if [ ${#missing[@]} -ne 0 ]; then
        print_error "Missing required tools: ${missing[*]}"
        exit 1
    fi

    if ! aws sts get-caller-identity &>/dev/null; then
        print_error "AWS credentials not configured. Run 'aws configure' or set AWS_PROFILE."
        exit 1
    fi

    print_success "All prerequisites OK."
}

# ── Terraform helpers ─────────────────────────────────────────────────────────
tf_init() {
    print_status "Initialising Terraform (environment: ${ENVIRONMENT})..."
    cd "$TERRAFORM_DIR"
    terraform init -upgrade \
        -backend-config="key=state/${ENVIRONMENT}/terraform.tfstate"
}

tf_vars() {
    echo "-var=environment=${ENVIRONMENT} -var=aws_region=${AWS_REGION} -var=project_name=${PROJECT_NAME} -var-file=terraform.${ENVIRONMENT}.tfvars"
}

tf_output() {
    cd "$TERRAFORM_DIR"
    terraform output -no-color -json 2>/dev/null \
        | jq -r --arg k "$1" '.[$k].value // empty' 2>/dev/null \
        || echo ""
}

# ── ECR login ─────────────────────────────────────────────────────────────────
ecr_login() {
    local ecr_url
    ecr_url=$(tf_output ecr_repository_url)
    if [ -z "$ecr_url" ]; then
        print_error "Could not read ECR URL from Terraform outputs. Run 'deploy infra' first." >&2
        exit 1
    fi
    local account_id
    account_id=$(echo "$ecr_url" | cut -d. -f1)
    print_status "Logging in to ECR (${account_id})..." >&2
    aws ecr get-login-password --region "$AWS_REGION" \
        | docker login --username AWS --password-stdin "$ecr_url" >&2
    echo "$ecr_url"
}

# ── Commands ──────────────────────────────────────────────────────────────────

cmd_plan() {
    check_prerequisites
    tf_init
    cd "$TERRAFORM_DIR"
    terraform plan $(tf_vars)
}

cmd_infra() {
    check_prerequisites
    tf_init
    cd "$TERRAFORM_DIR"
    print_status "Applying Terraform (environment: ${ENVIRONMENT})..."
    terraform apply $(tf_vars) -auto-approve
    print_success "Infrastructure deployed."
    cmd_info
}

cmd_build() {
    check_prerequisites
    local ecr_url
    ecr_url=$(ecr_login)

    print_status "Building Docker image (tag: ${IMAGE_TAG})..."
    cd "$(dirname "$0")/.."
    docker build -t "${PROJECT_NAME}:${IMAGE_TAG}" .

    print_status "Tagging and pushing to ECR..."
    docker tag "${PROJECT_NAME}:${IMAGE_TAG}" "${ecr_url}:${IMAGE_TAG}"
    docker push "${ecr_url}:${IMAGE_TAG}"

    print_success "Image pushed: ${ecr_url}:${IMAGE_TAG}"
}

cmd_deploy() {
    check_prerequisites

    # Build + push image
    cmd_build

    # Force ECS to pick up the new image
    local cluster service
    cluster=$(tf_output ecs_cluster_name)
    service=$(tf_output ecs_service_name)

    print_status "Forcing new ECS deployment (cluster: ${cluster}, service: ${service})..."
    aws ecs update-service \
        --cluster "$cluster" \
        --service "$service" \
        --force-new-deployment \
        --region "$AWS_REGION" \
        --output json | jq -r '.service.deployments[] | "\(.status): \(.runningCount)/\(.desiredCount) tasks"'

    print_success "Deployment triggered. Use './scripts/deploy.sh logs' to follow."
}

cmd_logs() {
    check_prerequisites
    local log_group
    log_group=$(tf_output log_group_name)
    if [ -z "$log_group" ]; then
        log_group="/ecs/${PROJECT_NAME}-${ENVIRONMENT}"
    fi
    print_status "Tailing logs from ${log_group} ..."
    aws logs tail "$log_group" --follow --region "$AWS_REGION"
}

cmd_status() {
    check_prerequisites
    local cluster service
    cluster=$(tf_output ecs_cluster_name)
    service=$(tf_output ecs_service_name)

    print_status "ECS service status (${cluster} / ${service}):"
    aws ecs describe-services \
        --cluster "$cluster" \
        --services "$service" \
        --region "$AWS_REGION" \
        --output json \
        | jq -r '.services[0] | "  Status:   \(.status)\n  Running:  \(.runningCount)/\(.desiredCount)\n  Pending:  \(.pendingCount)\n  Last event: \(.events[0].message)"'
}

cmd_info() {
    cd "$TERRAFORM_DIR"
    if [ ! -f ".terraform/terraform.tfstate" ] && [ ! -d ".terraform" ]; then
        print_warning "No Terraform state found. Run 'deploy infra' first."
        return
    fi

    local ecr_url cluster service log_group
    ecr_url=$(tf_output ecr_repository_url)
    cluster=$(tf_output ecs_cluster_name)
    service=$(tf_output ecs_service_name)
    log_group=$(tf_output log_group_name)

    echo
    print_success "=== Falke Deployment Info (${ENVIRONMENT}) ==="
    echo "  Region:      ${AWS_REGION}"
    echo "  ECR image:   ${ecr_url}:latest"
    echo "  ECS cluster: ${cluster}"
    echo "  ECS service: ${service}"
    echo "  Logs:        ${log_group}"
    echo
    echo "  Useful commands:"
    echo "    $0 deploy   -e ${ENVIRONMENT}   # build + push + restart"
    echo "    $0 logs     -e ${ENVIRONMENT}   # tail CloudWatch logs"
    echo "    $0 status   -e ${ENVIRONMENT}   # ECS service health"
    echo "    $0 destroy  -e ${ENVIRONMENT}   # tear down everything"
}

cmd_destroy() {
    check_prerequisites
    print_warning "This will DESTROY all infrastructure for environment '${ENVIRONMENT}'!"
    read -rp "Type the environment name to confirm (${ENVIRONMENT}): " confirm
    if [[ "$confirm" != "$ENVIRONMENT" ]]; then
        print_status "Cancelled."
        exit 0
    fi
    tf_init
    cd "$TERRAFORM_DIR"
    terraform destroy $(tf_vars) -auto-approve
    print_success "Infrastructure destroyed."
}

# ── Argument parsing ──────────────────────────────────────────────────────────
show_usage() {
    echo "Usage: $0 <command> [options]"
    echo
    echo "Commands:"
    echo "  plan      Terraform plan (dry-run)"
    echo "  infra     Apply Terraform infrastructure"
    echo "  build     Build Docker image and push to ECR"
    echo "  deploy    build + push + force new ECS deployment   (most common)"
    echo "  logs      Tail CloudWatch logs"
    echo "  status    Show ECS service health"
    echo "  info      Print deployment info / useful commands"
    echo "  destroy   Tear down all infrastructure"
    echo "  help      Show this message"
    echo
    echo "Options:"
    echo "  -e, --environment   dev | prod  [default: dev]"
    echo "  -r, --region        AWS region            [default: eu-west-2]"
    echo "  -t, --tag           Docker image tag       [default: latest]"
    echo
    echo "Examples:"
    echo "  $0 deploy  -e dev"
    echo "  $0 deploy  -e prod -t v1.2.3"
    echo "  $0 logs    -e dev"
    echo "  $0 destroy -e dev"
}

COMMAND=""
while [[ $# -gt 0 ]]; do
    case $1 in
        -e|--environment) ENVIRONMENT="$2"; shift 2 ;;
        -r|--region)      AWS_REGION="$2";  shift 2 ;;
        -t|--tag)         IMAGE_TAG="$2";   shift 2 ;;
        plan|infra|build|deploy|logs|status|info|destroy|help)
            COMMAND="$1"; shift ;;
        *)
            print_error "Unknown argument: $1"
            show_usage; exit 1 ;;
    esac
done

case "${COMMAND:-help}" in
    plan)    cmd_plan    ;;
    infra)   cmd_infra   ;;
    build)   cmd_build   ;;
    deploy)  cmd_deploy  ;;
    logs)    cmd_logs    ;;
    status)  cmd_status  ;;
    info)    cmd_info    ;;
    destroy) cmd_destroy ;;
    help|*)  show_usage  ;;
esac
