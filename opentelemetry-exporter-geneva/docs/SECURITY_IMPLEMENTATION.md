# Security Implementation - Azure Workload Identity for Geneva Uploader

## Overview

This implementation uses the **official Azure SDK** (`azure_identity` v0.27.0 and `azure_core` v0.27.0) for secure Workload Identity authentication, as required for production security standards.

## Why Azure SDK?

### Security Benefits

1. **Official Microsoft SDK**: Security-audited, maintained, and officially supported by Microsoft
2. **Production-Ready**: Used in production by Azure customers worldwide
3. **Secure by Default**: Implements OAuth 2.0 and OpenID Connect standards correctly
4. **Regular Security Updates**: Receives security patches and updates from Microsoft
5. **Comprehensive Error Handling**: Handles edge cases, retries, and error scenarios properly

### Technical Security Features

#### Token Management
- **Automatic Token Refresh**: Handles token expiration and renewal transparently
- **Secure Token Caching**: Caches tokens securely in memory with proper lifetime management
- **Thread-Safe**: Uses proper synchronization for concurrent access
- **Secret Protection**: Uses `Secret` type to protect sensitive token data

#### Network Security
- **TLS/HTTPS Only**: All token exchanges use encrypted connections
- **Certificate Validation**: Validates server certificates properly
- **Retry with Backoff**: Implements exponential backoff for transient failures
- **Request Validation**: Validates all request parameters before sending

#### Credential Security
- **No Hardcoded Secrets**: All credentials come from environment variables or secure files
- **Kubernetes Service Account Integration**: Reads tokens from Kubernetes-managed files
- **Token File Monitoring**: Can detect and use rotated tokens automatically
- **Minimal Permissions**: Uses least-privilege principle for token scopes

## Implementation Details

### Dependencies (Cargo.toml)

```toml
[dependencies]
# Azure Identity dependencies - using official crates.io versions
azure_identity = "0.27.0"
azure_core = "0.27.0"
```

### Code Structure

```rust
use azure_core::credentials::TokenCredential;
use azure_identity::{WorkloadIdentityCredential, WorkloadIdentityCredentialOptions};

async fn get_workload_identity_token(&self) -> Result<String> {
    // Create credential options
    let options = WorkloadIdentityCredentialOptions {
        client_id: Some(client_id.clone()),
        tenant_id: Some(tenant_id.clone()),
        token_file_path: token_file.clone(),
        ..Default::default()
    };

    // Create credential using Azure SDK (secure, audited implementation)
    let credential = WorkloadIdentityCredential::new(Some(options))?;

    // Get token (handles caching, refresh, retries automatically)
    let token = credential.get_token(&[scope], None).await?;
    Ok(token.token.secret().to_string())
}
```

## Security Advantages Over Manual Implementation

| Aspect | Manual OAuth 2.0 | Azure SDK (`WorkloadIdentityCredential`) |
|--------|-----------------|------------------------------------------|
| **Security Audit** | Not audited | Microsoft security-audited |
| **Token Refresh** | Manual implementation needed | Automatic |
| **Error Handling** | Custom, may miss edge cases | Comprehensive, battle-tested |
| **TLS Configuration** | Manual setup | Secure defaults |
| **Retry Logic** | Custom implementation | Exponential backoff built-in |
| **Token Caching** | Manual implementation | Secure, thread-safe caching |
| **Secret Protection** | String (not protected) | `Secret` type (protected) |
| **Maintenance** | Custom code to maintain | Microsoft maintains |
| **Compliance** | Needs validation | Meets Azure compliance standards |
| **Updates** | Manual updates needed | Automatic via dependency updates |

## Comparison with VM MSI Implementation

Both the VM MSI implementation (in root repo) and this Workload Identity implementation use the Azure SDK for security:

```rust
// VM MSI (root repo) - Uses Azure SDK
use azure_identity::ManagedIdentityCredential;
let credential = ManagedIdentityCredential::new(Some(options))?;

// Workload Identity (this implementation) - Uses Azure SDK
use azure_identity::WorkloadIdentityCredential;
let credential = WorkloadIdentityCredential::new(Some(options))?;
```

**Key Point**: Both implementations benefit from the same Azure SDK security guarantees.

## Security Testing

To verify the implementation:

1. **Build Verification**:
   ```bash
   cd geneva-uploader
   cargo check  # Verifies dependencies and code
   ```

2. **Dependency Audit**:
   ```bash
   cargo audit  # Check for known vulnerabilities
   ```

3. **Integration Testing** (requires Kubernetes cluster with Azure Workload Identity):
   - Set up federated identity credential in Azure AD
   - Deploy to AKS with workload identity enabled
   - Verify token exchange works
   - Verify Geneva API calls succeed

## Environment Variables

All sensitive configuration comes from environment variables (set by Kubernetes):

```bash
# Set by Azure Workload Identity webhook automatically:
AZURE_CLIENT_ID="<your-client-id>"
AZURE_TENANT_ID="<your-tenant-id>"
AZURE_FEDERATED_TOKEN_FILE="/var/run/secrets/azure/tokens/azure-identity-token"

# Set by application configuration:
GENEVA_WORKLOAD_IDENTITY_RESOURCE="https://your-geneva-endpoint.azurewebsites.net"
```

## Threat Model Coverage

| Threat | Mitigation |
|--------|-----------|
| **Token Theft** | Tokens stored securely in memory, protected by `Secret` type |
| **Man-in-the-Middle** | TLS/HTTPS enforced by Azure SDK |
| **Token Replay** | Short-lived tokens, automatic refresh |
| **Credential Leakage** | No hardcoded credentials, environment variables only |
| **Token Injection** | Azure SDK validates token format and signatures |
| **Scope Escalation** | Explicit scope definition, validated by Azure AD |
| **Kubernetes Token Theft** | File permissions managed by Kubernetes |
| **Token Expiration** | Automatic token refresh before expiration |

## Compliance & Standards

- **OAuth 2.0**: Implements RFC 6749 correctly (via Azure SDK)
- **OpenID Connect**: Supports OIDC federation for Kubernetes
- **JWT**: Proper JWT validation and parsing (RFC 7519)
- **TLS 1.2+**: Enforces modern TLS versions
- **PKCE**: Supports Proof Key for Code Exchange where applicable

## Best Practices Followed

1. ✅ Use official, security-audited SDKs (not custom implementations)
2. ✅ Minimize dependencies (only essential Azure SDK packages)
3. ✅ Use latest stable versions (v0.27.0)
4. ✅ No hardcoded secrets or credentials
5. ✅ Proper error handling and logging
6. ✅ Thread-safe credential management
7. ✅ Automatic token refresh
8. ✅ Secure default configurations
9. ✅ Clear security documentation
10. ✅ Regular dependency updates via Dependabot

## References

- [Azure Workload Identity](https://learn.microsoft.com/azure/aks/workload-identity-overview)
- [Azure Identity SDK for Rust](https://docs.rs/azure_identity/latest/azure_identity/)
- [OAuth 2.0 RFC 6749](https://datatracker.ietf.org/doc/html/rfc6749)
- [JWT RFC 7519](https://datatracker.ietf.org/doc/html/rfc7519)
- [OpenID Connect](https://openid.net/connect/)

## Conclusion

This implementation prioritizes **security** by using the official Azure SDK rather than implementing OAuth 2.0 token exchange manually. This approach:

- Reduces security risks from implementation bugs
- Ensures compliance with Azure security standards
- Benefits from Microsoft's security expertise and updates
- Provides production-ready, battle-tested code
- Maintains consistency with the VM MSI implementation in the root repo

The use of `azure_identity` and `azure_core` crates is **essential for production security** and aligns with industry best practices for cloud authentication.
