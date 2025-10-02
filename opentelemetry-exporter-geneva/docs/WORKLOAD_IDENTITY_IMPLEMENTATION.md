# Azure Workload Identity Support for Geneva Uploader

## Overview

This implementation adds Azure Workload Identity (Federated Identity) authentication support to the `geneva-uploader` crate in the `workload_identifier/opentelemetry-rust-contrib` repository. This allows Kubernetes workloads to authenticate with Geneva Config Service using service account tokens.

## Implementation Details

### 1. Authentication Method (`AuthMethod` enum)

Added a new `WorkloadIdentity` variant to the `AuthMethod` enum in `config_service/client.rs:50-71`:

```rust
AuthMethod::WorkloadIdentity {
    client_id: String,    // Azure AD Application (client) ID
    tenant_id: String,    // Azure AD Tenant ID
    token_file: Option<PathBuf>, // Path to service account token file
}
```

### 2. Error Handling

Added `WorkloadIdentityAuth` error variant to `GenevaConfigClientError` for workload identity-specific errors:

```rust
#[error("Workload Identity authentication error: {0}")]
WorkloadIdentityAuth(String),
```

### 3. Token Exchange Implementation

Implemented `get_workload_identity_token()` method (lines 335-397) that:

1. Uses the official Azure SDK `WorkloadIdentityCredential` for secure token exchange
2. Creates a `WorkloadIdentityCredentialOptions` with client ID, tenant ID, and optional token file path
3. Uses `WorkloadIdentityCredential::new()` to initialize the credential
4. Calls `get_token()` to exchange the Kubernetes service account token for an Azure AD access token
5. Returns the access token for use with Geneva Config Service

**Security Benefits:**
- Uses the official, security-audited `azure_identity` SDK
- Handles token refresh, retries, and error cases properly
- Follows Azure SDK best practices for credential management
- Automatically manages token caching and expiration

### 4. API Endpoint Selection

Workload Identity uses the `userapi` endpoint (similar to MSI), configured in the `new()` method (lines 281-288):

```rust
let api_path = match &config.auth_method {
    AuthMethod::Certificate { .. } => "api",
    AuthMethod::WorkloadIdentity { .. } => "userapi",
    ...
};
```

### 5. Request Authentication

Added Bearer token authentication in `fetch_ingestion_info()` method (lines 573-582):

```rust
match &self.config.auth_method {
    AuthMethod::WorkloadIdentity { .. } => {
        let token = self.get_workload_identity_token().await?;
        request = request.header(AUTHORIZATION, format!("Bearer {}", token));
    }
    ...
}
```

## Key Differences from VM-based MSI Implementation

| Aspect | VM MSI (root repo) | Workload Identity (this implementation) |
|--------|-------------------|----------------------------------------|
| **Dependencies** | Uses `azure_identity` and `azure_core` crates | Uses `azure_identity` and `azure_core` crates |
| **Token Source** | Azure Instance Metadata Service (IMDS) | Kubernetes service account token file |
| **Token Exchange** | `ManagedIdentityCredential` | `WorkloadIdentityCredential` |
| **Environment** | Azure VMs | Kubernetes clusters with Azure Workload Identity |
| **Configuration** | Client ID, Object ID, or Resource ID selector | Client ID, Tenant ID, and token file path |
| **Security** | Azure SDK managed | Azure SDK managed |

## Required Environment Variables

```bash
# Geneva configuration (same for both auth methods)
export GENEVA_ENDPOINT="https://your-geneva-endpoint.azurewebsites.net"
export GENEVA_ENVIRONMENT="Test"
export GENEVA_ACCOUNT="YourAccount"
export GENEVA_NAMESPACE="YourNamespace"
export GENEVA_REGION="eastus"
export GENEVA_CONFIG_MAJOR_VERSION=2

# Workload Identity specific
export MONITORING_GCS_AUTH_ID_TYPE="AuthWorkloadIdentity"
export GENEVA_WORKLOAD_IDENTITY_RESOURCE="https://your-geneva-endpoint.azurewebsites.net"

# Azure Workload Identity (Kubernetes sets these automatically)
export AZURE_CLIENT_ID="<your-client-id>"
export AZURE_TENANT_ID="<your-tenant-id>"
export AZURE_FEDERATED_TOKEN_FILE="/var/run/secrets/azure/tokens/azure-identity-token"
```

## Example Usage

See `examples/basic_workload_identity_test.rs` for a complete example:

```rust
let auth_method = AuthMethod::WorkloadIdentity {
    client_id: env::var("AZURE_CLIENT_ID").expect("AZURE_CLIENT_ID required"),
    tenant_id: env::var("AZURE_TENANT_ID").expect("AZURE_TENANT_ID required"),
    token_file: None, // Uses AZURE_FEDERATED_TOKEN_FILE by default
};

let config = GenevaClientConfig {
    endpoint,
    environment,
    account,
    namespace,
    region,
    config_major_version,
    tenant,
    role_name,
    role_instance,
    auth_method,
};

let geneva_client = GenevaClient::new(config).expect("Failed to create GenevaClient");
```

## Testing

Build and check the implementation:

```bash
cd workload_identifier/opentelemetry-rust-contrib/opentelemetry-exporter-geneva/geneva-uploader
cargo check

# Check the example
cd ../opentelemetry-exporter-geneva
cargo check --example basic_workload_identity_test
```

## Kubernetes Deployment

To use this in Kubernetes with Azure Workload Identity:

1. Set up Azure Workload Identity in your AKS cluster
2. Create a federated identity credential linking your Kubernetes service account to an Azure AD application
3. Configure your pod with the service account
4. The Azure Workload Identity webhook will automatically inject the required environment variables and mount the token file

Example Kubernetes deployment snippet:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: your-app-sa
  annotations:
    azure.workload.identity/client-id: "<your-client-id>"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: your-app
spec:
  template:
    metadata:
      labels:
        azure.workload.identity/use: "true"
    spec:
      serviceAccountName: your-app-sa
      containers:
      - name: your-app
        env:
        - name: MONITORING_GCS_AUTH_ID_TYPE
          value: "AuthWorkloadIdentity"
        - name: GENEVA_WORKLOAD_IDENTITY_RESOURCE
          value: "https://your-geneva-endpoint.azurewebsites.net"
        # Other Geneva env vars...
```

## Files Modified/Created

### Modified Files:
- `geneva-uploader/src/config_service/client.rs`:
  - Added `WorkloadIdentity` variant to `AuthMethod` enum
  - Added `WorkloadIdentityAuth` error variant
  - Implemented `get_workload_identity_token()` method
  - Updated `new()` to handle workload identity API path
  - Updated `fetch_ingestion_info()` to add Bearer token authentication
  - Added `AUTHORIZATION` header import

### Created Files:
- `opentelemetry-exporter-geneva/examples/basic_workload_identity_test.rs`:
  - Complete example demonstrating workload identity usage
  - Shows environment variable configuration
  - Demonstrates token exchange and Geneva logging

## Security Considerations

1. **Azure SDK Security**: Uses the official `azure_identity` crate (v0.27.0) which is security-audited and maintained by Microsoft
2. **Token File Security**: The Kubernetes service account token file is managed by Kubernetes and automatically rotated
3. **Token Lifetime**: Azure AD tokens have limited lifetime; `WorkloadIdentityCredential` handles token refresh and caching automatically
4. **No Secrets in Code**: All sensitive information (client ID, tenant ID) comes from environment variables or parameters
5. **TLS Required**: All token exchanges use HTTPS (enforced by Azure SDK)
6. **Credential Best Practices**: Follows Azure SDK patterns for secure credential management

## Future Enhancements

1. **Token Caching in Geneva Client**: Currently fetches a new Azure AD token for each Geneva API request; could cache the credential and reuse it (Azure SDK already handles internal token caching)
2. **Error Handling**: Add more granular error messages for different failure scenarios
3. **Metrics**: Add instrumentation for token exchange success/failure rates

**Note**: Retry logic and token refresh are already handled by the `azure_identity` SDK's `WorkloadIdentityCredential`.
