pub(crate) mod uploader;

#[cfg(test)]
mod tests {
    use crate::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
    use crate::{GenevaUploader, GenevaUploaderConfig};

    use std::env;
    use std::fs;
    use std::time::Instant;

    #[tokio::test]
    /// To run this test against a real Geneva Config Service and GIG, set the following environment variables:
    ///
    /// ```bash
    /// export GENEVA_ENDPOINT="https://<gcs-endpoint>"
    /// export GENEVA_ENVIRONMENT="Test"
    /// export GENEVA_ACCOUNT="YourAccountName"
    /// export GENEVA_NAMESPACE="YourNamespace"
    /// export GENEVA_REGION="YourRegion"
    /// export GENEVA_CONFIG_MAJOR_VERSION="2"
    /// export GENEVA_CERT_PATH="/path/to/client.p12"
    /// export GENEVA_CERT_PASSWORD="your-cert-password"
    /// export GENEVA_SOURCE_IDENTITY="Tenant=YourTenant/Role=YourRole/RoleInstance=YourInstance"
    /// export GENEVA_BLOB_PATH="/path/to/blob.bin"
    ///
    /// cargo test test_upload_to_gig_real_server -- --ignored --nocapture
    /// ```
    #[ignore]
    async fn test_upload_to_gig_real_server() {
        // === 1. Load binary blob ===
        let blob_path = env::var("GENEVA_BLOB_PATH").expect("GENEVA_BLOB_PATH env var is required");
        let data = fs::read(&blob_path).expect("Failed to read binary blob");
        println!("✅ Loaded blob from {} ({} bytes)", blob_path, data.len());

        // === 2. Read config from environment ===
        let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
        let environment = env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
        let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
        let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
        let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
        let cert_path = env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH is required");
        let cert_password = env::var("GENEVA_CERT_PASSWORD").unwrap_or_default();
        let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
            .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
            .parse::<u32>()
            .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");
        let source_identity = env::var("GENEVA_SOURCE_IDENTITY")
            .unwrap_or_else(|_| "Tenant=Default/Role=Uploader/RoleInstance=devhost".to_string());

        // === 3. Define uploader config ===
        let uploader_config = GenevaUploaderConfig {
            namespace: namespace.clone(),
            source_identity,
            environment: environment.clone(),
            schema_ids: None,
        };

        let config = GenevaConfigClientConfig {
            endpoint,
            environment,
            account,
            namespace,
            region,
            config_major_version,
            auth_method: AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            },
        };

        // === 4. Build client and uploader ===
        let config_client = GenevaConfigClient::new(config)
            .await
            .expect("Failed to create config client");
        let uploader = GenevaUploader::from_config_client(&config_client, uploader_config)
            .await
            .expect("Failed to create uploader");

        // === 5. Upload ===
        let event_name = "Log";
        let event_version = "Ver2v0";
        println!("🚀 Uploading to: {}", uploader.auth_info.endpoint);

        let start = Instant::now();
        let response = uploader
            .upload(data, event_name, event_version)
            .await
            .expect("Upload failed");

        println!(
            "✅ Upload complete in {:.2?}. Ticket: {}",
            start.elapsed(),
            response.ticket
        );
    }

    /// To run this test with parallel uploads:
    ///
    /// ```bash
    /// export GENEVA_ENDPOINT="https://<gcs-endpoint>"
    /// export GENEVA_ENVIRONMENT="Test"
    /// export GENEVA_ACCOUNT="YourAccount"
    /// export GENEVA_NAMESPACE="YourNamespace"
    /// export GENEVA_REGION="YourRegion"
    /// export GENEVA_CONFIG_MAJOR_VERSION="2"
    /// export GENEVA_CERT_PATH="/path/to/client.p12"
    /// export GENEVA_CERT_PASSWORD="your-password"
    /// export GENEVA_SOURCE_IDENTITY="Tenant=YourTenant/Role=Role/RoleInstance=Instance"
    /// export GENEVA_BLOB_PATH="/path/to/blob.bin"
    /// export GENEVA_PARALLEL_UPLOADS="10"
    ///
    /// cargo test test_parallel_uploads -- --ignored
    /// ```

    #[tokio::test]
    #[ignore]
    async fn test_parallel_uploads() {
        use crate::{
            AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, GenevaUploader,
            GenevaUploaderConfig,
        };
        use std::env;
        use std::fs;
        use std::time::Instant;

        // Read parallelism level from env
        let parallel_uploads: usize = env::var("GENEVA_PARALLEL_UPLOADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let blob_path = env::var("GENEVA_BLOB_PATH").expect("GENEVA_BLOB_PATH is required");
        let data = fs::read(&blob_path).expect("Failed to read blob");

        let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
        let environment = env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
        let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
        let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
        let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
        let cert_path = env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH is required");
        let cert_password = env::var("GENEVA_CERT_PASSWORD").unwrap_or_default();
        let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
            .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
            .parse::<u32>()
            .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");
        let source_identity = env::var("GENEVA_SOURCE_IDENTITY")
            .unwrap_or_else(|_| "Tenant=Default/Role=Uploader/RoleInstance=devhost".to_string());

        let uploader_config = GenevaUploaderConfig {
            namespace: namespace.clone(),
            source_identity,
            environment: environment.clone(),
            schema_ids: None,
        };

        let config = GenevaConfigClientConfig {
            endpoint,
            environment,
            account,
            namespace,
            region,
            config_major_version,
            auth_method: AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            },
        };

        let config_client = GenevaConfigClient::new(config)
            .await
            .expect("Failed to create config client");
        let uploader = GenevaUploader::from_config_client(&config_client, uploader_config)
            .await
            .expect("Failed to create uploader");

        let event_name = "Log";
        let event_version = "Ver2v0";

        println!("🚀 Launching {parallel_uploads} parallel uploads...");

        let start_all = Instant::now();

        let mut handles = vec![];
        for i in 0..parallel_uploads {
            let uploader = uploader.clone();
            let data = data.clone();
            let event_name = event_name.to_string();
            let event_version = event_version.to_string();

            let handle = tokio::spawn(async move {
                let start = Instant::now();
                match uploader.upload(data, &event_name, &event_version).await {
                    Ok(resp) => {
                        let elapsed = start.elapsed();
                        println!(
                            "✅ Upload {} complete in {:.2?}. Ticket: {}",
                            i, elapsed, resp.ticket
                        );
                        Some(elapsed)
                    }
                    Err(e) => {
                        eprintln!("❌ Upload {} failed: {:?}", i, e);
                        None
                    }
                }
            });
            handles.push(handle);
        }

        let durations: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .filter_map(|res| res.ok().flatten())
            .collect();

        let total_time = start_all.elapsed();

        if !durations.is_empty() {
            let avg =
                durations.iter().map(|d| d.as_secs_f64()).sum::<f64>() / durations.len() as f64;
            println!("📊 Average upload duration: {:.2} sec", avg);
        }

        println!(
            "⏱️ Total elapsed for {} parallel uploads: {:.2?}",
            parallel_uploads, total_time
        );
    }
}
