# Geneva Exporter - Workload Identity Example

This example demonstrates how to use Azure Workload Identity to authenticate to Geneva Config Service (GCS) from an Azure Kubernetes Service (AKS) cluster.

## Prerequisites

- Azure CLI (`az`) installed and authenticated
- `kubectl` configured to access your AKS cluster
- AKS cluster with OIDC Issuer and Workload Identity enabled
- Azure Container Registry (ACR) attached to your AKS cluster
- Access to Geneva/Jarvis portal for registering managed identities

## Architecture

Azure Workload Identity enables Kubernetes pods to authenticate to Azure services using **User-Assigned Managed Identities** with federated identity credentials. This approach uses Managed Identities, NOT App Registrations, simplifying credential management.

**Authentication Flow**:
1. Pod runs with a Kubernetes service account
2. Kubernetes injects a service account JWT token into the pod
3. Application exchanges the Kubernetes token for an Azure AD access token using the Managed Identity
4. Azure AD access token is used to authenticate to Geneva Config Service

**Key Difference**: Traditional Workload Identity setups often use App Registrations with client secrets. This implementation uses **User-Assigned Managed Identities** instead, which eliminates the need to manage secrets or certificates.

## Step 1: Enable Workload Identity on AKS (if not already enabled)

```bash
# Check if OIDC issuer is enabled
az aks show --resource-group <resource-group> --name <cluster-name> --query "oidcIssuerProfile.issuerUrl" -o tsv

# If not enabled, enable it
az aks update \
  --resource-group <resource-group> \
  --name <cluster-name> \
  --enable-oidc-issuer \
  --enable-workload-identity
```

## Step 2: Create User-Assigned Managed Identity

**Important**: We create a **User-Assigned Managed Identity**, NOT an Azure AD App Registration. Workload Identity with Managed Identities is simpler and doesn't require managing client secrets or certificates.

```bash
# Set variables
RESOURCE_GROUP="<your-resource-group>"
LOCATION="<azure-region>"  # e.g., eastus2
IDENTITY_NAME="geneva-uploader-identity-$(openssl rand -hex 3)"

# Create the managed identity (NOT an App Registration)
az identity create \
  --resource-group $RESOURCE_GROUP \
  --name $IDENTITY_NAME \
  --location $LOCATION

# Get the client ID and principal ID
export AZURE_CLIENT_ID=$(az identity show --resource-group $RESOURCE_GROUP --name $IDENTITY_NAME --query clientId -o tsv)
export PRINCIPAL_ID=$(az identity show --resource-group $RESOURCE_GROUP --name $IDENTITY_NAME --query principalId -o tsv)

echo "Client ID: $AZURE_CLIENT_ID"
echo "Principal ID: $PRINCIPAL_ID"

# Note: The AZURE_CLIENT_ID here is the managed identity's client ID, not an App Registration
```

## Step 3: Create Kubernetes Service Account

```bash
# Set Kubernetes variables
NAMESPACE="default"  # or your preferred namespace
SERVICE_ACCOUNT_NAME="geneva-uploader-sa"

# Create service account with workload identity annotation
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: ServiceAccount
metadata:
  name: $SERVICE_ACCOUNT_NAME
  namespace: $NAMESPACE
  annotations:
    azure.workload.identity/client-id: $AZURE_CLIENT_ID
EOF
```

## Step 4: Create Federated Identity Credential

```bash
# Get AKS OIDC issuer URL
export AKS_OIDC_ISSUER=$(az aks show --resource-group $RESOURCE_GROUP --name <cluster-name> --query "oidcIssuerProfile.issuerUrl" -o tsv)

# Create federated credential
FEDERATED_CREDENTIAL_NAME="geneva-fedcred-$(openssl rand -hex 3)"

az identity federated-credential create \
  --name $FEDERATED_CREDENTIAL_NAME \
  --identity-name $IDENTITY_NAME \
  --resource-group $RESOURCE_GROUP \
  --issuer $AKS_OIDC_ISSUER \
  --subject system:serviceaccount:$NAMESPACE:$SERVICE_ACCOUNT_NAME \
  --audience api://AzureADTokenExchange

echo "Federated credential created: $FEDERATED_CREDENTIAL_NAME"
```

## Step 5: Register Managed Identity in Geneva Portal

Register the managed identity using the **Principal ID (Object ID)** from Step 2. Wait 5-10 minutes for propagation.

## Step 6: Get Your Azure Tenant ID

```bash
export AZURE_TENANT_ID=$(az account show --query tenantId -o tsv)
echo "Tenant ID: $AZURE_TENANT_ID"
```

## Step 7: Build and Push Docker Image

```bash
# Navigate to the workspace root
cd /path/to/opentelemetry-rust-contrib

# Set ACR variables
ACR_NAME="<your-acr-name>"
IMAGE_NAME="geneva-uploader-workload-identity-test"
IMAGE_TAG="latest"

# Build the image
docker build \
  -f opentelemetry-exporter-geneva/opentelemetry-exporter-geneva/examples/Dockerfile \
  -t $ACR_NAME.azurecr.io/$IMAGE_NAME:$IMAGE_TAG \
  .

# Push to ACR
az acr login --name $ACR_NAME
docker push $ACR_NAME.azurecr.io/$IMAGE_NAME:$IMAGE_TAG
```

## Step 8: Create ConfigMap with Geneva Configuration

```bash
# Create ConfigMap with your Geneva environment configuration
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: ConfigMap
metadata:
  name: geneva-config
  namespace: $NAMESPACE
data:
  GENEVA_ENDPOINT: "https://abc.com"  # Use your Geneva endpoint
  GENEVA_ENVIRONMENT: "Test"  # Your environment name
  GENEVA_ACCOUNT: "PipelineAgent2Demo"  # Your Geneva account
  GENEVA_NAMESPACE: "PAdemo2"  # Your Geneva namespace
  GENEVA_REGION: "eastus"  # Your Azure region
  GENEVA_CONFIG_MAJOR_VERSION: "2"
  MONITORING_GCS_AUTH_ID_TYPE: "AuthWorkloadIdentity"
  GENEVA_WORKLOAD_IDENTITY_RESOURCE: "https://monitor.azure.com"  # Azure Public Cloud
  GENEVA_TENANT: "default-tenant"
  GENEVA_ROLE_NAME: "default-role"
  GENEVA_ROLE_INSTANCE: "default-instance"
EOF
```

### Resource URI for Different Azure Clouds

The `GENEVA_WORKLOAD_IDENTITY_RESOURCE` value depends on your Azure cloud:

- **Azure Public Cloud**: `https://monitor.azure.com`
- **Azure Government**: `https://monitor.azure.us`
- **Azure China**: `https://monitor.azure.cn`

## Step 9: Deploy the Application

```bash
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: Pod
metadata:
  name: geneva-uploader-test
  namespace: $NAMESPACE
  labels:
    azure.workload.identity/use: "true"
spec:
  serviceAccountName: $SERVICE_ACCOUNT_NAME
  containers:
  - name: geneva-uploader
    image: $ACR_NAME.azurecr.io/$IMAGE_NAME:$IMAGE_TAG
    envFrom:
    - configMapRef:
        name: geneva-config
    env:
    - name: AZURE_CLIENT_ID
      value: "$AZURE_CLIENT_ID"
    - name: AZURE_TENANT_ID
      value: "$AZURE_TENANT_ID"
  restartPolicy: Never
EOF
```

**Important - Environment Variable Setup:**

The pod spec above sets these environment variables:
- `AZURE_CLIENT_ID` - **Must be set explicitly in pod spec** (as shown above)
- `AZURE_TENANT_ID` - **Must be set explicitly in pod spec** (as shown above)

The workload identity webhook automatically injects:
- `AZURE_FEDERATED_TOKEN_FILE` - Auto-injected by webhook, points to `/var/run/secrets/azure/tokens/azure-identity-token`
- Projected service account token volume mount

These three environment variables are automatically read by the Azure Identity SDK (`azure_identity` crate) at runtime. The Geneva client does not need to be configured with these values - they are discovered from the environment.

## Step 10: Verify the Deployment

```bash
# Check pod status
kubectl get pod geneva-uploader-test -n $NAMESPACE

# View pod logs
kubectl logs geneva-uploader-test -n $NAMESPACE

# Check workload identity injection
kubectl describe pod geneva-uploader-test -n $NAMESPACE | grep -A 5 "Environment:"
```

Expected log output should show:
- Successful token acquisition from Azure AD
- Connection to Geneva Config Service
- Log events being sent
- "Shutting down provider" after 30 seconds

## Environment Variables Reference

### Required Variables (from ConfigMap)

| Variable | Description | Example |
|----------|-------------|---------|
| `GENEVA_ENDPOINT` | Geneva Config Service endpoint URL | `https://gcs.ppe.monitoring.core.windows.net` |
| `GENEVA_ENVIRONMENT` | Environment name in Geneva | `Test` |
| `GENEVA_ACCOUNT` | Geneva monitoring account | `PipelineAgent2Demo` |
| `GENEVA_NAMESPACE` | Geneva namespace | `PAdemo2` |
| `GENEVA_REGION` | Azure region | `eastus` |
| `GENEVA_CONFIG_MAJOR_VERSION` | Config schema version | `2` |
| `MONITORING_GCS_AUTH_ID_TYPE` | Authentication type | `AuthWorkloadIdentity` |
| `GENEVA_WORKLOAD_IDENTITY_RESOURCE` | Azure Monitor resource URI | `https://monitor.azure.com` |

### Required Variables (from Pod Spec / Auto-injected)

These variables are set in the pod environment and automatically read by the Azure Identity SDK:

| Variable | Description | Source |
|----------|-------------|--------|
| `AZURE_CLIENT_ID` | Managed identity client ID | Set in pod spec (Step 9) |
| `AZURE_TENANT_ID` | Azure AD tenant ID | Set in pod spec (Step 9) |
| `AZURE_FEDERATED_TOKEN_FILE` | Path to Kubernetes token file | Auto-injected by workload identity webhook |

### Optional Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `GENEVA_TENANT` | Geneva tenant identifier | `default-tenant` |
| `GENEVA_ROLE_NAME` | Role name for Geneva | `default-role` |
| `GENEVA_ROLE_INSTANCE` | Role instance identifier | `default-instance` |
| `WORKLOAD_IDENTITY_TOKEN_FILE` | Custom token file path | Uses `AZURE_FEDERATED_TOKEN_FILE` |

## Troubleshooting

### Pod fails with "AZURE_CLIENT_ID required" or similar Azure Identity errors

**Cause**: Azure Identity SDK cannot find required environment variables.

**Fix**: Ensure:
- `AZURE_CLIENT_ID` and `AZURE_TENANT_ID` are set in pod spec (Step 9)
- Pod has label `azure.workload.identity/use: "true"`
- Service account has annotation `azure.workload.identity/client-id: <client-id>`
- Workload identity webhook is running in the cluster (it injects `AZURE_FEDERATED_TOKEN_FILE`)

### Token exchange fails with "invalid_client"

**Cause**: Federated credential not configured correctly.

**Fix**: Verify:
- Federated credential issuer matches AKS OIDC issuer exactly
- Subject is `system:serviceaccount:<namespace>:<service-account-name>`
- Audience is `api://AzureADTokenExchange`

### "Invalid scope" error

**Cause**: Wrong resource URI for your Azure cloud.

**Fix**: Update `GENEVA_WORKLOAD_IDENTITY_RESOURCE` in ConfigMap:
- Azure Public: `https://monitor.azure.com`
- Azure Government: `https://monitor.azure.us`
- Azure China: `https://monitor.azure.cn`

### Logs show success but no data in Geneva

**Possible causes**:
1. Managed identity not registered in Geneva (wait 5-10 minutes after registration)
2. Identity doesn't have correct permissions in Geneva account
3. Wrong Geneva endpoint or account configuration

**Fix**:
- Verify identity in Geneva portal
- Check Geneva account permissions
- Review ConfigMap values against Geneva documentation

### Check workload identity webhook status

```bash
kubectl get pods -n kube-system | grep workload-identity
kubectl logs -n kube-system -l app.kubernetes.io/name=workload-identity-webhook
```

## Example kubectl Commands

```bash
# Watch pod status
kubectl get pod geneva-uploader-test -n $NAMESPACE -w

# Get detailed pod info
kubectl describe pod geneva-uploader-test -n $NAMESPACE

# Stream logs
kubectl logs -f geneva-uploader-test -n $NAMESPACE

# Check service account
kubectl get serviceaccount $SERVICE_ACCOUNT_NAME -n $NAMESPACE -o yaml

# Check ConfigMap
kubectl get configmap geneva-config -n $NAMESPACE -o yaml

# Delete and redeploy
kubectl delete pod geneva-uploader-test -n $NAMESPACE
# Then re-run Step 9
```

## Cleanup

```bash
# Delete Kubernetes resources
kubectl delete pod geneva-uploader-test -n $NAMESPACE
kubectl delete configmap geneva-config -n $NAMESPACE
kubectl delete serviceaccount $SERVICE_ACCOUNT_NAME -n $NAMESPACE

# Delete Azure resources
az identity federated-credential delete \
  --name $FEDERATED_CREDENTIAL_NAME \
  --identity-name $IDENTITY_NAME \
  --resource-group $RESOURCE_GROUP

az identity delete \
  --resource-group $RESOURCE_GROUP \
  --name $IDENTITY_NAME

# Remove from Jarvis (Geneva portal) manually
```

## References

- [Azure Workload Identity Documentation](https://azure.github.io/azure-workload-identity/)
- [AKS Workload Identity Overview](https://learn.microsoft.com/azure/aks/workload-identity-overview)

