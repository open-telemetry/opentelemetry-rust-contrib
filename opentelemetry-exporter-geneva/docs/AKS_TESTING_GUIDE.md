# Testing Geneva Uploader with Azure Workload Identity on AKS

This guide walks you through testing the Workload Identity implementation on Azure Kubernetes Service (AKS).

## 🚀 Quick Start (TL;DR)

If you already have an AKS cluster with Workload Identity enabled:

```bash
# 1. Set your configuration
export ACR_NAME="yourregistry"
export APP_NAME="geneva-uploader-wi"
export SERVICE_ACCOUNT_NAME="geneva-uploader-sa"
export APPLICATION_CLIENT_ID="<your-app-client-id>"
export AZURE_TENANT_ID="<your-tenant-id>"
export AKS_OIDC_ISSUER="<your-aks-oidc-issuer>"

# 2. Build and push image
cd /tmp/tbd2/workload_identifier/opentelemetry-rust-contrib
az acr build --registry "${ACR_NAME}" --image geneva-uploader-test:latest --file Dockerfile .

# 3. Create service account
kubectl apply -f - <<EOF
apiVersion: v1
kind: ServiceAccount
metadata:
  name: ${SERVICE_ACCOUNT_NAME}
  annotations:
    azure.workload.identity/client-id: "${APPLICATION_CLIENT_ID}"
EOF

# 4. Create federated credential
az ad app federated-credential create \
  --id "${APPLICATION_CLIENT_ID}" \
  --parameters "{\"name\":\"${APP_NAME}-fedcred\",\"issuer\":\"${AKS_OIDC_ISSUER}\",\"subject\":\"system:serviceaccount:default:${SERVICE_ACCOUNT_NAME}\",\"audiences\":[\"api://AzureADTokenExchange\"]}"

# 5. Create config and deploy
kubectl create configmap geneva-config \
  --from-literal=GENEVA_ENDPOINT="https://your-endpoint.net" \
  --from-literal=GENEVA_ENVIRONMENT="Test" \
  --from-literal=GENEVA_ACCOUNT="YourAccount" \
  --from-literal=GENEVA_NAMESPACE="YourNamespace" \
  --from-literal=GENEVA_REGION="eastus" \
  --from-literal=GENEVA_CONFIG_MAJOR_VERSION="2" \
  --from-literal=GENEVA_WORKLOAD_IDENTITY_RESOURCE="https://your-endpoint.net" \
  --from-literal=MONITORING_GCS_AUTH_ID_TYPE="AuthWorkloadIdentity"

kubectl run geneva-test --image=${ACR_NAME}.azurecr.io/geneva-uploader-test:latest \
  --labels=azure.workload.identity/use=true \
  --serviceaccount=${SERVICE_ACCOUNT_NAME} \
  --env-from=configmap/geneva-config

# 6. Check logs
kubectl logs -f geneva-test
```

---

## Prerequisites

- Azure CLI installed and configured
- kubectl installed
- An Azure subscription with permissions to create resources
- Docker installed (for building container images)
- Azure Container Registry (ACR) or Docker Hub account

## Table of Contents

1. [Azure Setup](#1-azure-setup)
2. [AKS Cluster Setup](#2-aks-cluster-setup)
3. [Build and Push Container Image](#3-build-and-push-container-image)
4. [Configure Azure Workload Identity](#4-configure-azure-workload-identity)
5. [Deploy to AKS](#5-deploy-to-aks)
6. [Verify and Test](#6-verify-and-test)
7. [Troubleshooting](#7-troubleshooting)

---

## 1. Azure Setup

### 1.1 Set Environment Variables

```bash
# Azure Configuration
export AZURE_SUBSCRIPTION_ID="<your-subscription-id>"
export AZURE_RESOURCE_GROUP="geneva-workload-identity-rg"
export AZURE_LOCATION="eastus"
export AKS_CLUSTER_NAME="geneva-test-cluster"
export ACR_NAME="genevauploader"  # Must be globally unique

# Azure AD Application
export APP_NAME="geneva-uploader-workload-identity"
export SERVICE_ACCOUNT_NAME="geneva-uploader-sa"
export SERVICE_ACCOUNT_NAMESPACE="default"

# Geneva Configuration (replace with your values)
export GENEVA_ENDPOINT="https://your-geneva-endpoint.azurewebsites.net"
export GENEVA_ENVIRONMENT="Test"
export GENEVA_ACCOUNT="YourAccount"
export GENEVA_NAMESPACE="YourNamespace"
export GENEVA_REGION="eastus"
export GENEVA_CONFIG_MAJOR_VERSION="2"
```

### 1.2 Login to Azure

```bash
az login
az account set --subscription "${AZURE_SUBSCRIPTION_ID}"
```

### 1.3 Create Resource Group

```bash
az group create \
  --name "${AZURE_RESOURCE_GROUP}" \
  --location "${AZURE_LOCATION}"
```

---

## 2. AKS Cluster Setup

### 2.1 Create Azure Container Registry (ACR)

```bash
az acr create \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${ACR_NAME}" \
  --sku Basic

# Login to ACR
az acr login --name "${ACR_NAME}"
```

### 2.2 Create AKS Cluster with Workload Identity

```bash
# Create AKS cluster with workload identity enabled
az aks create \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --node-count 2 \
  --enable-oidc-issuer \
  --enable-workload-identity \
  --attach-acr "${ACR_NAME}" \
  --generate-ssh-keys

# Get AKS credentials
az aks get-credentials \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --overwrite-existing

# Verify cluster access
kubectl get nodes
```

### 2.3 Get OIDC Issuer URL

```bash
export AKS_OIDC_ISSUER=$(az aks show \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --query "oidcIssuerProfile.issuerUrl" \
  --output tsv)

echo "OIDC Issuer: ${AKS_OIDC_ISSUER}"
```

---

## 3. Build and Push Container Image

### 3.1 Create Dockerfile

Create a `Dockerfile` in the example directory:

```dockerfile
# Dockerfile for Geneva Uploader Workload Identity Test
FROM rust:1.85-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the entire workspace
COPY . .

# Build the example
WORKDIR /app/opentelemetry-exporter-geneva/opentelemetry-exporter-geneva
RUN cargo build --release --example basic_workload_identity_test

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary
COPY --from=builder \
    /app/opentelemetry-exporter-geneva/opentelemetry-exporter-geneva/target/release/examples/basic_workload_identity_test \
    /usr/local/bin/geneva-uploader-test

# Run as non-root user
RUN useradd -m -u 1000 appuser
USER appuser

ENTRYPOINT ["/usr/local/bin/geneva-uploader-test"]
```

### 3.2 Build and Push Image

```bash
# Navigate to the repo root
cd /tmp/tbd2/workload_identifier/opentelemetry-rust-contrib

# Build the Docker image
docker build -t "${ACR_NAME}.azurecr.io/geneva-uploader-test:latest" -f Dockerfile .

# Push to ACR
docker push "${ACR_NAME}.azurecr.io/geneva-uploader-test:latest"
```

**Alternative: Build directly in ACR**

```bash
# Build in ACR (no local Docker required)
az acr build \
  --registry "${ACR_NAME}" \
  --image geneva-uploader-test:latest \
  --file Dockerfile \
  .
```

---

## 4. Configure Azure Workload Identity

### 4.1 Create Azure AD Application and Service Principal

```bash
# Create Azure AD application
export APPLICATION_CLIENT_ID=$(az ad app create \
  --display-name "${APP_NAME}" \
  --query appId \
  --output tsv)

echo "Application Client ID: ${APPLICATION_CLIENT_ID}"

# Create service principal
az ad sp create --id "${APPLICATION_CLIENT_ID}"

# Get tenant ID
export AZURE_TENANT_ID=$(az account show --query tenantId --output tsv)
echo "Tenant ID: ${AZURE_TENANT_ID}"
```

### 4.2 Grant Permissions to Geneva Resources

If Geneva requires specific Azure permissions (e.g., access to storage accounts or other resources):

```bash
# Example: Grant Storage Blob Data Contributor role (adjust as needed)
# Replace <GENEVA_RESOURCE_ID> with your actual Geneva resource ID

# az role assignment create \
#   --role "Storage Blob Data Contributor" \
#   --assignee "${APPLICATION_CLIENT_ID}" \
#   --scope "<GENEVA_RESOURCE_ID>"
```

### 4.3 Create Kubernetes Service Account

```bash
# Create namespace if not using default
# kubectl create namespace "${SERVICE_ACCOUNT_NAMESPACE}"

# Create service account
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: ServiceAccount
metadata:
  name: ${SERVICE_ACCOUNT_NAME}
  namespace: ${SERVICE_ACCOUNT_NAMESPACE}
  annotations:
    azure.workload.identity/client-id: "${APPLICATION_CLIENT_ID}"
EOF
```

### 4.4 Create Federated Identity Credential

```bash
# Create federated identity credential
az identity federated-credential create \
  --name "${APP_NAME}-federated-credential" \
  --identity-name "${APP_NAME}" \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --issuer "${AKS_OIDC_ISSUER}" \
  --subject "system:serviceaccount:${SERVICE_ACCOUNT_NAMESPACE}:${SERVICE_ACCOUNT_NAME}" \
  --audiences "api://AzureADTokenExchange"
```

**Note**: If the above command fails because you created an app registration (not a managed identity), use this instead:

```bash
az ad app federated-credential create \
  --id "${APPLICATION_CLIENT_ID}" \
  --parameters "{
    \"name\": \"${APP_NAME}-federated-credential\",
    \"issuer\": \"${AKS_OIDC_ISSUER}\",
    \"subject\": \"system:serviceaccount:${SERVICE_ACCOUNT_NAMESPACE}:${SERVICE_ACCOUNT_NAME}\",
    \"audiences\": [\"api://AzureADTokenExchange\"]
  }"
```

---

## 5. Deploy to AKS

### 5.1 Create ConfigMap for Geneva Configuration

```bash
kubectl create configmap geneva-config \
  --from-literal=GENEVA_ENDPOINT="${GENEVA_ENDPOINT}" \
  --from-literal=GENEVA_ENVIRONMENT="${GENEVA_ENVIRONMENT}" \
  --from-literal=GENEVA_ACCOUNT="${GENEVA_ACCOUNT}" \
  --from-literal=GENEVA_NAMESPACE="${GENEVA_NAMESPACE}" \
  --from-literal=GENEVA_REGION="${GENEVA_REGION}" \
  --from-literal=GENEVA_CONFIG_MAJOR_VERSION="${GENEVA_CONFIG_MAJOR_VERSION}" \
  --from-literal=GENEVA_WORKLOAD_IDENTITY_RESOURCE="${GENEVA_ENDPOINT}" \
  --from-literal=MONITORING_GCS_AUTH_ID_TYPE="AuthWorkloadIdentity" \
  --namespace="${SERVICE_ACCOUNT_NAMESPACE}"
```

### 5.2 Create Deployment Manifest

Create `deployment.yaml`:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: geneva-uploader-test
  namespace: default
  labels:
    app: geneva-uploader-test
spec:
  replicas: 1
  selector:
    matchLabels:
      app: geneva-uploader-test
  template:
    metadata:
      labels:
        app: geneva-uploader-test
        azure.workload.identity/use: "true"  # Enable workload identity injection
    spec:
      serviceAccountName: geneva-uploader-sa
      containers:
      - name: geneva-uploader
        image: ${ACR_NAME}.azurecr.io/geneva-uploader-test:latest
        imagePullPolicy: Always
        envFrom:
        - configMapRef:
            name: geneva-config
        env:
        # Optional: Override default values
        - name: GENEVA_TENANT
          value: "default-tenant"
        - name: GENEVA_ROLE_NAME
          value: "test-role"
        - name: GENEVA_ROLE_INSTANCE
          value: "test-instance-1"
        resources:
          requests:
            memory: "128Mi"
            cpu: "100m"
          limits:
            memory: "256Mi"
            cpu: "200m"
---
apiVersion: v1
kind: Pod
metadata:
  name: geneva-uploader-test-pod
  namespace: default
  labels:
    app: geneva-uploader-test
    azure.workload.identity/use: "true"
spec:
  serviceAccountName: geneva-uploader-sa
  containers:
  - name: geneva-uploader
    image: ${ACR_NAME}.azurecr.io/geneva-uploader-test:latest
    envFrom:
    - configMapRef:
        name: geneva-config
  restartPolicy: Never
```

**Apply with environment variable substitution:**

```bash
# Replace environment variables in the YAML
envsubst < deployment.yaml | kubectl apply -f -
```

**Or apply directly:**

```bash
cat <<EOF | kubectl apply -f -
apiVersion: apps/v1
kind: Deployment
metadata:
  name: geneva-uploader-test
  namespace: default
  labels:
    app: geneva-uploader-test
spec:
  replicas: 1
  selector:
    matchLabels:
      app: geneva-uploader-test
  template:
    metadata:
      labels:
        app: geneva-uploader-test
        azure.workload.identity/use: "true"
    spec:
      serviceAccountName: ${SERVICE_ACCOUNT_NAME}
      containers:
      - name: geneva-uploader
        image: ${ACR_NAME}.azurecr.io/geneva-uploader-test:latest
        imagePullPolicy: Always
        envFrom:
        - configMapRef:
            name: geneva-config
        resources:
          requests:
            memory: "128Mi"
            cpu: "100m"
          limits:
            memory: "256Mi"
            cpu: "200m"
EOF
```

---

## 6. Verify and Test

### 6.1 Check Workload Identity Webhook Injection

```bash
# Get the pod name
export POD_NAME=$(kubectl get pods -l app=geneva-uploader-test -o jsonpath='{.items[0].metadata.name}')

# Verify workload identity environment variables are injected
kubectl exec -it "${POD_NAME}" -- env | grep AZURE

# Expected output:
# AZURE_CLIENT_ID=<your-client-id>
# AZURE_TENANT_ID=<your-tenant-id>
# AZURE_FEDERATED_TOKEN_FILE=/var/run/secrets/azure/tokens/azure-identity-token
# AZURE_AUTHORITY_HOST=https://login.microsoftonline.com/
```

### 6.2 Verify Token File Mount

```bash
# Check if the token file is mounted
kubectl exec -it "${POD_NAME}" -- ls -la /var/run/secrets/azure/tokens/

# Read the token (it's a JWT)
kubectl exec -it "${POD_NAME}" -- cat /var/run/secrets/azure/tokens/azure-identity-token
```

### 6.3 Check Application Logs

```bash
# View pod logs
kubectl logs -f "${POD_NAME}"

# Expected output should include:
# - Successful token acquisition from Azure AD
# - Successful connection to Geneva Config Service
# - Log messages being sent to Geneva
# - No authentication errors
```

### 6.4 Check Pod Events

```bash
# Check for any errors or warnings
kubectl describe pod "${POD_NAME}"

# Look for events like:
# - Successfully pulled image
# - Started container
# - No error events
```

### 6.5 Test Token Exchange Manually (Debug)

```bash
# Exec into the pod
kubectl exec -it "${POD_NAME}" -- /bin/bash

# Inside the pod, manually test token acquisition
curl -X POST "https://login.microsoftonline.com/${AZURE_TENANT_ID}/oauth2/v2.0/token" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "client_id=${AZURE_CLIENT_ID}" \
  -d "client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer" \
  -d "client_assertion=$(cat /var/run/secrets/azure/tokens/azure-identity-token)" \
  -d "scope=${GENEVA_WORKLOAD_IDENTITY_RESOURCE}/.default" \
  -d "grant_type=client_credentials"

# Should return a JSON response with access_token
```

---

## 7. Troubleshooting

### 7.1 Common Issues

#### Issue: Pod doesn't start / ImagePullBackOff

```bash
# Check if ACR is attached to AKS
az aks show \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --query "servicePrincipalProfile.clientId" \
  --output tsv

# Attach ACR if needed
az aks update \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --attach-acr "${ACR_NAME}"
```

#### Issue: AZURE_CLIENT_ID not injected

```bash
# Verify workload identity is enabled on cluster
az aks show \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --query "oidcIssuerProfile.enabled" \
  --output tsv

# Should output: true

# Check if pod has the required label
kubectl get pod "${POD_NAME}" -o jsonpath='{.metadata.labels.azure\.workload\.identity/use}'

# Should output: true
```

#### Issue: Token file not mounted

```bash
# Check if workload identity webhook is running
kubectl get pods -n kube-system | grep workload-identity

# Restart the pod to trigger injection
kubectl delete pod "${POD_NAME}"
```

#### Issue: "Failed to create WorkloadIdentityCredential" error

```bash
# Verify service account has the annotation
kubectl get serviceaccount "${SERVICE_ACCOUNT_NAME}" -o yaml

# Should show:
# annotations:
#   azure.workload.identity/client-id: <your-client-id>

# Verify federated credential exists
az ad app federated-credential list \
  --id "${APPLICATION_CLIENT_ID}" \
  --output table
```

#### Issue: "Token acquisition failed" error

```bash
# Check federated credential configuration
az ad app federated-credential show \
  --id "${APPLICATION_CLIENT_ID}" \
  --federated-credential-id "${APP_NAME}-federated-credential"

# Verify:
# 1. Issuer matches AKS OIDC issuer
# 2. Subject matches "system:serviceaccount:<namespace>:<service-account-name>"
# 3. Audiences includes "api://AzureADTokenExchange"
```

#### Issue: Geneva API authentication fails

```bash
# Verify GENEVA_WORKLOAD_IDENTITY_RESOURCE is set correctly
kubectl exec -it "${POD_NAME}" -- env | grep GENEVA_WORKLOAD_IDENTITY_RESOURCE

# Verify the scope/resource URL is correct for your Geneva instance
# Try with and without trailing slash or /.default suffix
```

### 7.2 Enable Debug Logging

Update deployment to enable debug logging:

```yaml
env:
- name: RUST_LOG
  value: "debug"
- name: RUST_BACKTRACE
  value: "1"
```

Apply changes:

```bash
kubectl apply -f deployment.yaml
kubectl rollout restart deployment geneva-uploader-test
```

### 7.3 Verify Network Connectivity

```bash
# Test connectivity to Azure AD
kubectl exec -it "${POD_NAME}" -- curl -v https://login.microsoftonline.com

# Test connectivity to Geneva endpoint
kubectl exec -it "${POD_NAME}" -- curl -v "${GENEVA_ENDPOINT}"
```

---

## 8. Clean Up

### 8.1 Delete Kubernetes Resources

```bash
kubectl delete deployment geneva-uploader-test
kubectl delete serviceaccount "${SERVICE_ACCOUNT_NAME}"
kubectl delete configmap geneva-config
```

### 8.2 Delete Azure Resources

```bash
# Delete federated credential
az ad app federated-credential delete \
  --id "${APPLICATION_CLIENT_ID}" \
  --federated-credential-id "${APP_NAME}-federated-credential"

# Delete service principal
az ad sp delete --id "${APPLICATION_CLIENT_ID}"

# Delete app registration
az ad app delete --id "${APPLICATION_CLIENT_ID}"

# Delete AKS cluster
az aks delete \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${AKS_CLUSTER_NAME}" \
  --yes --no-wait

# Delete ACR
az acr delete \
  --resource-group "${AZURE_RESOURCE_GROUP}" \
  --name "${ACR_NAME}" \
  --yes

# Delete resource group
az group delete \
  --name "${AZURE_RESOURCE_GROUP}" \
  --yes --no-wait
```

---

## 9. Production Considerations

### 9.1 Security Best Practices

1. **Use Managed Identity**: Consider using Azure User-Assigned Managed Identity instead of App Registration
2. **Least Privilege**: Grant only necessary permissions to the service principal
3. **Secrets Management**: Use Azure Key Vault for sensitive configuration
4. **Network Policies**: Implement Kubernetes Network Policies to restrict traffic
5. **Pod Security**: Use Pod Security Standards (restricted profile)

### 9.2 Monitoring

```bash
# Add Prometheus annotations for monitoring
metadata:
  annotations:
    prometheus.io/scrape: "true"
    prometheus.io/port: "8080"
    prometheus.io/path: "/metrics"
```

### 9.3 High Availability

```yaml
spec:
  replicas: 3
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxUnavailable: 1
      maxSurge: 1
```

---

## References

- [Azure Workload Identity Documentation](https://learn.microsoft.com/azure/aks/workload-identity-overview)
- [AKS Workload Identity Quick Start](https://azure.github.io/azure-workload-identity/docs/quick-start.html)
- [Azure Identity SDK for Rust](https://docs.rs/azure_identity/latest/azure_identity/)
- [Troubleshooting Workload Identity](https://azure.github.io/azure-workload-identity/docs/troubleshooting.html)

---

## Quick Reference Commands

```bash
# Get pod logs
kubectl logs -f <pod-name>

# Exec into pod
kubectl exec -it <pod-name> -- /bin/bash

# Describe pod
kubectl describe pod <pod-name>

# Check service account
kubectl get serviceaccount <sa-name> -o yaml

# Restart deployment
kubectl rollout restart deployment geneva-uploader-test

# View pod environment variables
kubectl exec <pod-name> -- env | sort
```
