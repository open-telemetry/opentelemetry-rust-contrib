#!/bin/bash

# Geneva FFI Examples Environment Setup Script
# Run this before running the C or Go examples

echo "Geneva FFI Examples Environment Setup"
echo "====================================="
echo ""

# Set required environment variables
export GENEVA_ENDPOINT="https://abc.azurewebsites.net"
export GENEVA_ENVIRONMENT="Test"
export GENEVA_ACCOUNT="myaccount"
export GENEVA_NAMESPACE="myns"
export GENEVA_REGION="eastus"
export GENEVA_CONFIG_MAJOR_VERSION="2"

# Optional variables with defaults
export GENEVA_TENANT="default-tenant"
export GENEVA_ROLE_NAME="default-role"
export GENEVA_ROLE_INSTANCE="default-instance"

echo "✅ Environment variables set:"
echo "   GENEVA_ENDPOINT=$GENEVA_ENDPOINT"
echo "   GENEVA_ENVIRONMENT=$GENEVA_ENVIRONMENT"
echo "   GENEVA_ACCOUNT=$GENEVA_ACCOUNT"
echo "   GENEVA_NAMESPACE=$GENEVA_NAMESPACE"
echo "   GENEVA_REGION=$GENEVA_REGION"
echo "   GENEVA_CONFIG_MAJOR_VERSION=$GENEVA_CONFIG_MAJOR_VERSION"
echo "   GENEVA_TENANT=$GENEVA_TENANT"
echo "   GENEVA_ROLE_NAME=$GENEVA_ROLE_NAME"
echo "   GENEVA_ROLE_INSTANCE=$GENEVA_ROLE_INSTANCE"
echo ""

echo "🔐 Authentication Method: Managed Identity (default)"
echo ""
echo "📝 For Certificate Authentication, also set:"
echo "   export GENEVA_CERT_PATH=\"/path/to/your/cert.p12\""
echo "   export GENEVA_CERT_PASSWORD=\"your-cert-password\""
echo ""

echo "🚀 Now you can run the examples:"
echo "   C Example:  ./c_resourcelogs_example"
echo "   Go Example: go run go_async_sync_example.go"
echo ""

echo "💡 These environment variables are now set in your current shell session."
echo "   To make them permanent, add them to your ~/.bashrc or ~/.zshrc"
