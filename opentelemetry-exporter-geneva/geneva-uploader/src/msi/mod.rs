//! Azure Managed Service Identity (MSI) authentication module
//! 
//! This module provides MSI authentication functionality integrated directly into the Geneva uploader.
//! It contains the essential components from the MSI library needed for Geneva authentication.

#[cfg(feature = "msi_auth")]
pub mod error;
#[cfg(feature = "msi_auth")]
pub mod ffi;
#[cfg(feature = "msi_auth")]
pub mod token_source;
#[cfg(feature = "msi_auth")]
pub mod types;

#[cfg(feature = "msi_auth")]
pub use error::{MsiError, MsiResult};
#[cfg(feature = "msi_auth")]
pub use token_source::get_msi_access_token;
#[cfg(feature = "msi_auth")]
pub use types::ManagedIdentity;

/// Azure Monitor service endpoints for Geneva authentication
#[cfg(feature = "msi_auth")]
pub mod resources {
    /// Azure Monitor endpoint for public Azure cloud (used for Geneva authentication)
    pub const AZURE_MONITOR_PUBLIC: &str = "https://monitor.core.windows.net/";
    // Add more endpoints as needed
}

#[cfg(test)]
#[cfg(feature = "msi_auth")]
mod tests {
    use super::*;

    /// Test MSI authentication against real Azure endpoints
    /// 
    /// This test is ignored by default and requires environment variables to be set:
    /// - TEST_MSI_OBJECT_ID: Object ID of the managed identity
    /// - TEST_MSI_CLIENT_ID: Client ID of the managed identity (optional, alternative to object ID)
    /// - TEST_MSI_RESOURCE_ID: Resource ID of the managed identity (optional, alternative to object/client ID)
    /// 
    /// Run with: cargo test --features msi_auth test_real_msi_authentication -- --ignored
    /// 
    /// Note: This test only works when running on Azure infrastructure where the managed identity is assigned
    #[tokio::test]
    #[ignore = "Requires real Azure environment and managed identity setup"]
    async fn test_real_msi_authentication() {
        use std::env;
        
        // Try to get identity configuration from environment variables
        let identity = if let Ok(object_id) = env::var("TEST_MSI_OBJECT_ID") {
            Some(ManagedIdentity::ObjectId(object_id))
        } else if let Ok(client_id) = env::var("TEST_MSI_CLIENT_ID") {
            Some(ManagedIdentity::ClientId(client_id))
        } else if let Ok(resource_id) = env::var("TEST_MSI_RESOURCE_ID") {
            Some(ManagedIdentity::ResourceId(resource_id))
        } else {
            // Test with system-assigned identity if no user-assigned identity is specified
            None
        };

        println!("Testing MSI authentication with identity: {:?}", identity);

        // Test getting MSI token for Azure Monitor
        let result = get_msi_access_token(
            resources::AZURE_MONITOR_PUBLIC,
            identity.as_ref(),
            false, // not AntMds
        );

        match result {
            Ok(token) => {
                println!("✅ Successfully obtained MSI token");
                println!("Token prefix: {}...", &token[..std::cmp::min(20, token.len())]);
                
                // Validate that we got a JWT token (should have 3 parts separated by dots)
                let parts: Vec<&str> = token.split('.').collect();
                assert_eq!(parts.len(), 3, "Token should be a valid JWT with 3 parts");
                
                // Validate token is not empty
                assert!(!token.is_empty(), "Token should not be empty");
                assert!(token.len() > 100, "Token should be substantial length");
            }
            Err(e) => {
                panic!("❌ MSI authentication failed: {}", e);
            }
        }
    }

    /// Test MSI authentication with explicit fallback behavior
    /// 
    /// This test demonstrates how fallback from user-assigned to system-assigned identity works.
    /// Requires environment variables:
    /// - TEST_MSI_OBJECT_ID: Object ID of a user-assigned managed identity
    /// 
    /// Run with: cargo test --features msi_auth test_msi_fallback_behavior -- --ignored
    #[tokio::test]
    #[ignore = "Requires real Azure environment and managed identity setup"]
    async fn test_msi_fallback_behavior() {
        use std::env;
        
        let object_id = env::var("TEST_MSI_OBJECT_ID")
            .expect("TEST_MSI_OBJECT_ID environment variable must be set for this test");
        
        let user_assigned_identity = ManagedIdentity::ObjectId(object_id);
        
        println!("Testing user-assigned identity: {:?}", user_assigned_identity);
        
        // Test with user-assigned identity
        let result = get_msi_access_token(
            resources::AZURE_MONITOR_PUBLIC,
            Some(&user_assigned_identity),
            false,
        );
        
        match result {
            Ok(token) => {
                println!("✅ User-assigned identity authentication successful");
                println!("Token prefix: {}...", &token[..std::cmp::min(20, token.len())]);
            }
            Err(e) => {
                println!("⚠️  User-assigned identity failed: {}", e);
                
                // Try fallback to system-assigned identity
                println!("Testing fallback to system-assigned identity...");
                let fallback_result = get_msi_access_token(
                    resources::AZURE_MONITOR_PUBLIC,
                    None, // system-assigned
                    false,
                );
                
                match fallback_result {
                    Ok(token) => {
                        println!("✅ Fallback to system-assigned identity successful");
                        println!("Token prefix: {}...", &token[..std::cmp::min(20, token.len())]);
                    }
                    Err(fallback_e) => {
                        panic!("❌ Both user-assigned and system-assigned identity failed. User-assigned error: {}, System-assigned error: {}", e, fallback_e);
                    }
                }
            }
        }
    }

    /// Test MSI token validation and parsing
    /// 
    /// This test validates that MSI tokens have the expected JWT structure
    /// and contain the necessary claims.
    /// 
    /// Run with: cargo test --features msi_auth test_msi_token_validation -- --ignored
    #[tokio::test]
    #[ignore = "Requires real Azure environment and managed identity setup"]
    async fn test_msi_token_validation() {
        use std::env;
        
        // Use any available identity for this test
        let identity = if let Ok(object_id) = env::var("TEST_MSI_OBJECT_ID") {
            Some(ManagedIdentity::ObjectId(object_id))
        } else {
            None // system-assigned
        };
        
        let token = get_msi_access_token(
            resources::AZURE_MONITOR_PUBLIC,
            identity.as_ref(),
            false,
        ).expect("Failed to get MSI token");
        
        // Validate JWT structure
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "Token should have 3 parts (header.payload.signature)");
        
        // Validate each part is base64-encoded and not empty
        assert!(!parts[0].is_empty(), "JWT header should not be empty");
        assert!(!parts[1].is_empty(), "JWT payload should not be empty");
        assert!(!parts[2].is_empty(), "JWT signature should not be empty");
        
        // Decode and validate payload contains expected claims
        use base64::{engine::general_purpose, Engine as _};
        
        let payload = parts[1];
        // Add padding if necessary
        let payload_padded = match payload.len() % 4 {
            0 => payload.to_string(),
            2 => format!("{payload}=="),
            3 => format!("{payload}="),
            _ => payload.to_string(),
        };
        
        let decoded = general_purpose::URL_SAFE_NO_PAD
            .decode(&payload_padded)
            .expect("Failed to decode JWT payload");
        
        let payload_str = String::from_utf8(decoded)
            .expect("JWT payload should be valid UTF-8");
        
        let payload_json: serde_json::Value = serde_json::from_str(&payload_str)
            .expect("JWT payload should be valid JSON");
        
        // Validate required claims
        assert!(payload_json.get("aud").is_some(), "Token should have 'aud' (audience) claim");
        assert!(payload_json.get("iss").is_some(), "Token should have 'iss' (issuer) claim");
        assert!(payload_json.get("exp").is_some(), "Token should have 'exp' (expiration) claim");
        
        // Validate audience matches Azure Monitor
        let audience = payload_json["aud"].as_str()
            .expect("Audience claim should be a string");
        assert!(
            audience.contains("monitor.core.windows.net") || audience.contains("monitor.azure.com"),
            "Audience should be for Azure Monitor service, got: {}", audience
        );
        
        println!("✅ Token validation successful");
        println!("Audience: {}", audience);
        println!("Issuer: {}", payload_json["iss"].as_str().unwrap_or("N/A"));
    }

    /// Test MSI authentication against real Geneva Config Service (GCS)
    /// 
    /// This test validates end-to-end MSI authentication with Geneva Config Service
    /// to retrieve ingestion gateway information, similar to certificate-based tests.
    /// 
    /// Required environment variables:
    /// - GENEVA_ENDPOINT: Geneva Config Service endpoint URL
    /// - GENEVA_ENVIRONMENT: Environment name (e.g., "production", "dev")
    /// - GENEVA_ACCOUNT: Geneva account name
    /// - GENEVA_NAMESPACE: Service namespace
    /// - GENEVA_REGION: Azure region (e.g., "westus2")
    /// - GENEVA_CONFIG_MAJOR_VERSION: Configuration major version (e.g., "1")
    /// - TEST_MSI_OBJECT_ID: Object ID of managed identity (optional)
    /// - TEST_MSI_CLIENT_ID: Client ID of managed identity (optional, alternative to object ID)
    /// - TEST_MSI_RESOURCE_ID: Resource ID of managed identity (optional, alternative to object/client ID)
    /// 
    /// Run with: cargo test --features msi_auth test_gcs_msi_authentication -- --ignored
    /// 
    /// Note: Must run on Azure infrastructure where the managed identity is assigned
    #[tokio::test]
    #[ignore = "Requires real Azure environment and Geneva Config Service access"]
    async fn test_gcs_msi_authentication() {
        use std::env;
        use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, MsiIdentityType};

        // Read Geneva configuration from environment variables
        let endpoint = env::var("GENEVA_ENDPOINT")
            .expect("GENEVA_ENDPOINT environment variable must be set");
        let environment = env::var("GENEVA_ENVIRONMENT")
            .expect("GENEVA_ENVIRONMENT environment variable must be set");
        let account = env::var("GENEVA_ACCOUNT")
            .expect("GENEVA_ACCOUNT environment variable must be set");
        let namespace = env::var("GENEVA_NAMESPACE")
            .expect("GENEVA_NAMESPACE environment variable must be set");
        let region = env::var("GENEVA_REGION")
            .expect("GENEVA_REGION environment variable must be set");
        let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
            .expect("GENEVA_CONFIG_MAJOR_VERSION environment variable must be set")
            .parse::<u32>()
            .expect("GENEVA_CONFIG_MAJOR_VERSION must be a valid unsigned integer");

        // Determine which identity to use based on environment variables
        let auth_method = if let Ok(object_id) = env::var("TEST_MSI_OBJECT_ID") {
            println!("Using user-assigned managed identity with Object ID: {}", object_id);
            AuthMethod::ManagedIdentity {
                identity: Some(MsiIdentityType::ObjectId(object_id)),
                fallback_to_default: true,
            }
        } else if let Ok(client_id) = env::var("TEST_MSI_CLIENT_ID") {
            println!("Using user-assigned managed identity with Client ID: {}", client_id);
            AuthMethod::ManagedIdentity {
                identity: Some(MsiIdentityType::ClientId(client_id)),
                fallback_to_default: true,
            }
        } else if let Ok(resource_id) = env::var("TEST_MSI_RESOURCE_ID") {
            println!("Using user-assigned managed identity with Resource ID: {}", resource_id);
            AuthMethod::ManagedIdentity {
                identity: Some(MsiIdentityType::ResourceId(resource_id)),
                fallback_to_default: true,
            }
        } else {
            println!("Using system-assigned managed identity (no user-assigned identity specified)");
            AuthMethod::ManagedIdentity {
                identity: None,
                fallback_to_default: false,
            }
        };

        let config = GenevaConfigClientConfig {
            endpoint,
            environment,
            account,
            namespace,
            region,
            config_major_version,
            auth_method,
        };

        println!("Connecting to Geneva Config Service with MSI authentication...");
        let client = GenevaConfigClient::new(config)
            .expect("Failed to create Geneva Config client with MSI authentication");

        println!("Fetching ingestion info from GCS...");
        let (ingestion_info, moniker_info, token_endpoint) = client
            .get_ingestion_info()
            .await
            .expect("Failed to get ingestion info from GCS using MSI authentication");

        // Validate the response contains expected fields
        assert!(
            !ingestion_info.endpoint.is_empty(),
            "Ingestion endpoint should not be empty"
        );
        assert!(
            !ingestion_info.auth_token.is_empty(),
            "Auth token should not be empty"
        );
        assert!(
            !moniker_info.name.is_empty(),
            "Moniker name should not be empty"
        );
        assert!(
            !moniker_info.account_group.is_empty(),
            "Moniker account group should not be empty"
        );
        assert!(
            !token_endpoint.is_empty(),
            "Token endpoint should not be empty"
        );

        // Validate that we got a proper JWT token
        let token_parts: Vec<&str> = ingestion_info.auth_token.split('.').collect();
        assert_eq!(
            token_parts.len(),
            3,
            "Auth token should be a valid JWT with 3 parts"
        );

        // Validate moniker is a diagnostic moniker (should contain "diag")
        assert!(
            moniker_info.name.contains("diag"),
            "Moniker name should contain 'diag', got: {}",
            moniker_info.name
        );

        println!("✅ Successfully authenticated with Geneva Config Service using MSI");
        println!("Ingestion endpoint: {}", ingestion_info.endpoint);
        println!("Token endpoint: {}", token_endpoint);
        println!("Auth token length: {}", ingestion_info.auth_token.len());
        println!("Auth token expiry: {}", ingestion_info.auth_token_expiry_time);
        println!("Moniker name: {}", moniker_info.name);
        println!("Moniker account group: {}", moniker_info.account_group);
    }

    /// Test MSI vs Certificate authentication comparison
    /// 
    /// This test compares the results of MSI authentication vs certificate authentication
    /// to ensure they provide equivalent access to Geneva Config Service.
    /// 
    /// Required environment variables (same as previous tests):
    /// - All Geneva Config variables (GENEVA_ENDPOINT, etc.)
    /// - Certificate variables: GENEVA_CERT_PATH, GENEVA_CERT_PASSWORD
    /// - MSI identity variables: TEST_MSI_OBJECT_ID (or alternatives)
    /// 
    /// Run with: cargo test --features msi_auth test_msi_vs_certificate_authentication -- --ignored
    #[tokio::test]
    #[ignore = "Requires real Azure environment with both certificate and MSI access"]
    async fn test_msi_vs_certificate_authentication() {
        use std::env;
        use std::path::PathBuf;
        use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, MsiIdentityType};

        // Read common Geneva configuration
        let endpoint = env::var("GENEVA_ENDPOINT")
            .expect("GENEVA_ENDPOINT environment variable must be set");
        let environment = env::var("GENEVA_ENVIRONMENT")
            .expect("GENEVA_ENVIRONMENT environment variable must be set");
        let account = env::var("GENEVA_ACCOUNT")
            .expect("GENEVA_ACCOUNT environment variable must be set");
        let namespace = env::var("GENEVA_NAMESPACE")
            .expect("GENEVA_NAMESPACE environment variable must be set");
        let region = env::var("GENEVA_REGION")
            .expect("GENEVA_REGION environment variable must be set");
        let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
            .expect("GENEVA_CONFIG_MAJOR_VERSION environment variable must be set")
            .parse::<u32>()
            .expect("GENEVA_CONFIG_MAJOR_VERSION must be a valid unsigned integer");

        // Test 1: Certificate authentication
        println!("🔑 Testing certificate authentication...");
        let cert_path = env::var("GENEVA_CERT_PATH")
            .expect("GENEVA_CERT_PATH environment variable must be set for comparison test");
        let cert_password = env::var("GENEVA_CERT_PASSWORD")
            .expect("GENEVA_CERT_PASSWORD environment variable must be set for comparison test");

        let cert_config = GenevaConfigClientConfig {
            endpoint: endpoint.clone(),
            environment: environment.clone(),
            account: account.clone(),
            namespace: namespace.clone(),
            region: region.clone(),
            config_major_version,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(cert_path),
                password: cert_password,
            },
        };

        let cert_client = GenevaConfigClient::new(cert_config)
            .expect("Failed to create certificate-based client");

        let (cert_ingestion_info, cert_moniker_info, cert_token_endpoint) = cert_client
            .get_ingestion_info()
            .await
            .expect("Failed to get ingestion info using certificate authentication");

        // Test 2: MSI authentication
        println!("🔐 Testing MSI authentication...");
        let msi_identity = if let Ok(object_id) = env::var("TEST_MSI_OBJECT_ID") {
            Some(MsiIdentityType::ObjectId(object_id))
        } else if let Ok(client_id) = env::var("TEST_MSI_CLIENT_ID") {
            Some(MsiIdentityType::ClientId(client_id))
        } else if let Ok(resource_id) = env::var("TEST_MSI_RESOURCE_ID") {
            Some(MsiIdentityType::ResourceId(resource_id))
        } else {
            None // system-assigned
        };

        let msi_config = GenevaConfigClientConfig {
            endpoint,
            environment,
            account,
            namespace,
            region,
            config_major_version,
            auth_method: AuthMethod::ManagedIdentity {
                identity: msi_identity,
                fallback_to_default: true,
            },
        };

        let msi_client = GenevaConfigClient::new(msi_config)
            .expect("Failed to create MSI-based client");

        let (msi_ingestion_info, msi_moniker_info, msi_token_endpoint) = msi_client
            .get_ingestion_info()
            .await
            .expect("Failed to get ingestion info using MSI authentication");

        // Test 3: Compare results
        println!("🔍 Comparing authentication results...");

        // Both should provide access to the same Geneva account/namespace
        assert_eq!(
            cert_moniker_info.name,
            msi_moniker_info.name,
            "Certificate and MSI should access the same moniker"
        );
        assert_eq!(
            cert_moniker_info.account_group,
            msi_moniker_info.account_group,
            "Certificate and MSI should access the same account group"
        );

        // Token endpoints should be the same (from JWT parsing)
        assert_eq!(
            cert_token_endpoint,
            msi_token_endpoint,
            "Certificate and MSI should provide tokens for the same endpoint"
        );

        // Both tokens should be valid JWTs
        let cert_token_parts: Vec<&str> = cert_ingestion_info.auth_token.split('.').collect();
        let msi_token_parts: Vec<&str> = msi_ingestion_info.auth_token.split('.').collect();
        assert_eq!(cert_token_parts.len(), 3, "Certificate token should be valid JWT");
        assert_eq!(msi_token_parts.len(), 3, "MSI token should be valid JWT");

        println!("✅ Both authentication methods provide equivalent access");
        println!("Certificate ingestion endpoint: {}", cert_ingestion_info.endpoint);
        println!("MSI ingestion endpoint: {}", msi_ingestion_info.endpoint);
        println!("Shared moniker: {}", cert_moniker_info.name);
        println!("Shared token endpoint: {}", cert_token_endpoint);
        println!("Certificate token length: {}", cert_ingestion_info.auth_token.len());
        println!("MSI token length: {}", msi_ingestion_info.auth_token.len());
    }
}
