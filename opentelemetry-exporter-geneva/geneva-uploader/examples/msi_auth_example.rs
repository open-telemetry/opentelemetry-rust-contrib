//! Example demonstrating MSI (Managed Service Identity) authentication with Geneva uploader
//!
//! This example shows how to configure the Geneva client to use Azure Managed Identity
//! for authentication instead of certificate-based authentication.
//!
//! Run with: cargo run --example msi_auth_example

use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig, MsiIdentityType};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Geneva MSI Authentication Example");
    println!("=================================");

    // Example 1: System-assigned managed identity
    println!("\n1. System-assigned Managed Identity");
    let system_config = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::ManagedIdentity {
            identity: None, // System-assigned identity
            fallback_to_default: false,
        },
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: None,
    };

    println!("Config: {:?}", system_config);

    // Alternative using builder method
    let system_config_builder = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::Certificate {
            path: "placeholder.p12".into(),
            password: "placeholder".to_string(),
        }, // Will be overridden
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: None,
    }
    .with_system_assigned_identity();

    println!("Builder config: {:?}", system_config_builder);

    // Example 2: User-assigned managed identity by Client ID
    println!("\n2. User-assigned Managed Identity (Client ID)");
    let user_config = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::Certificate {
            path: "placeholder.p12".into(),
            password: "placeholder".to_string(),
        }, // Will be overridden
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: None,
    }
    .with_user_assigned_client_id("12345678-1234-1234-1234-123456789012".to_string(), true);

    println!("User-assigned config: {:?}", user_config);

    // Example 3: User-assigned managed identity by Object ID
    println!("\n3. User-assigned Managed Identity (Object ID)");
    let object_id_config = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::Certificate {
            path: "placeholder.p12".into(),
            password: "placeholder".to_string(),
        }, // Will be overridden
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: None,
    }
    .with_user_assigned_object_id("87654321-4321-4321-4321-210987654321".to_string(), false);

    println!("Object ID config: {:?}", object_id_config);

    // Example 4: User-assigned managed identity by Resource ID
    println!("\n4. User-assigned Managed Identity (Resource ID)");
    let resource_id_config = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::Certificate {
            path: "placeholder.p12".into(),
            password: "placeholder".to_string(),
        }, // Will be overridden
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: None,
    }
    .with_user_assigned_resource_id(
        "/subscriptions/sub-id/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/my-identity".to_string(),
        false,
    );

    println!("Resource ID config: {:?}", resource_id_config);

    // Example 5: Manual construction with MsiIdentityType
    println!("\n5. Manual MSI Configuration");
    let manual_config = GenevaClientConfig {
        endpoint: "https://geneva.example.com".to_string(),
        environment: "prod".to_string(),
        account: "myaccount".to_string(),
        namespace: "myservice".to_string(),
        region: "westus2".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::ManagedIdentity {
            identity: Some(MsiIdentityType::ClientId(
                "abcdef12-3456-7890-abcd-ef1234567890".to_string(),
            )),
            fallback_to_default: true, // Fallback to system-assigned if user-assigned fails
        },
        tenant: "mytenant".to_string(),
        role_name: "myrole".to_string(),
        role_instance: "instance1".to_string(),
        max_concurrent_uploads: Some(8), // Custom concurrency
    };

    println!("Manual config: {:?}", manual_config);

    // Note: In a real application, you would create the client and use it like this:
    // 
    // let client = GenevaClient::new(system_config).await?;
    // 
    // // Create some sample log data
    // let log_record = LogRecord {
    //     body: Some("Test log message".into()),
    //     severity_text: "INFO".to_string(),
    //     attributes: vec![],
    //     ..Default::default()
    // };
    // 
    // let scope_logs = ScopeLogs {
    //     log_records: vec![log_record],
    //     ..Default::default()
    // };
    // 
    // let resource_logs = ResourceLogs {
    //     scope_logs: vec![scope_logs],
    //     ..Default::default()
    // };
    // 
    // // Upload the logs
    // client.upload_logs(&[resource_logs]).await?;

    println!("\n✅ MSI configuration examples completed successfully!");
    println!("\nKey Points:");
    println!("• System-assigned identity: Use `identity: None`");
    println!("• User-assigned identity: Use `identity: Some(MsiIdentityType::...)`");
    println!("• Fallback option: Set `fallback_to_default: true` to try system-assigned if user-assigned fails");
    println!("• Builder methods: Use `.with_system_assigned_identity()`, `.with_user_assigned_client_id()`, etc.");
    println!("• The MSI library handles token acquisition and refresh automatically");
    println!("• Existing token caching and expiry logic is reused for MSI tokens");

    Ok(())
}
