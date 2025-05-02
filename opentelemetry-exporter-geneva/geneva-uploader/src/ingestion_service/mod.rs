pub(crate) mod uploader;

#[cfg(test)]
mod tests {
    use std::time::Instant;

    mod test_helpers {
        use crate::{
            AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, GenevaUploader,
            GenevaUploaderConfig,
        };
        use std::env;
        use std::fs;
        use std::sync::Arc;

        pub struct TestUploadContext {
            pub data: Vec<u8>,
            pub uploader: GenevaUploader,
            pub event_name: String,
            pub event_version: String,
        }

        pub async fn build_test_upload_context() -> TestUploadContext {
            // Load binary blob
            let blob_path =
                env::var("GENEVA_BLOB_PATH").expect("GENEVA_BLOB_PATH env var is required");
            let data = fs::read(&blob_path).expect("Failed to read binary blob");

            // Read config from environment
            let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
            let environment =
                env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
            let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
            let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
            let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
            let cert_path = std::path::PathBuf::from(
                std::env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH is required"),
            );
            let cert_password = env::var("GENEVA_CERT_PASSWORD").unwrap_or_default();
            let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
                .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
                .parse::<u32>()
                .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");
            let source_identity = env::var("GENEVA_SOURCE_IDENTITY").unwrap_or_else(|_| {
                "Tenant=Default/Role=Uploader/RoleInstance=devhost".to_string()
            });
            let schema_ids =
                "c1ce0ecea020359624c493bbe97f9e80;0da22cabbee419e000541a5eda732eb3".to_string();

            // Define uploader config
            let uploader_config = GenevaUploaderConfig {
                namespace: namespace.clone(),
                source_identity,
                environment: environment.clone(),
                schema_ids,
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

            // Build client and uploader
            let config_client =
                GenevaConfigClient::new(config).expect("Failed to create config client");
            let uploader =
                GenevaUploader::from_config_client(Arc::new(config_client), uploader_config)
                    .await
                    .expect("Failed to create uploader");

            // Event name/version
            let event_name = "Log".to_string();
            let event_version = "Ver2v0".to_string();

            TestUploadContext {
                data,
                uploader,
                event_name,
                event_version,
            }
        }
    }

    #[tokio::test]
    /// To run this test against a real Geneva Config Service and GIG, set the following environment variables:
    ///
    /// ```bash
    /// export GENEVA_ENDPOINT="xxxhttps://<gcs-endpoint>"
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
        let ctx = test_helpers::build_test_upload_context().await;
        println!("✅ Loaded blob ({} bytes)", ctx.data.len());
        // below call is only for logging purposes, to get endpoint and auth info.
        let (auth_info, _, _) = ctx
            .uploader
            .config_client
            .get_ingestion_info()
            .await
            .unwrap();
        println!("🚀 Uploading to: {}", auth_info.endpoint);

        let start = Instant::now();
        let response = ctx
            .uploader
            .upload(ctx.data, &ctx.event_name, &ctx.event_version)
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
    /// cargo test test_parallel_uploads -- --ignored --nocapture
    /// Output:
    // 🔥 Performing warm-up upload...
    // 🔥 Warm-up upload complete in 222.42ms
    // 🚀 Launching 5 parallel uploads...
    // ✅ Upload 2 complete in 120.43ms. Ticket: ...
    // ✅ Upload 4 complete in 120.35ms. Ticket: ...
    // ✅ Upload 3 complete in 120.50ms. Ticket: ...
    // ✅ Upload 1 complete in 154.62ms. Ticket: ...
    // ✅ Upload 0 complete in 154.65ms. Ticket: ...
    // 📊 Average upload duration: 133.60 ms
    // ⏱️ Total elapsed for 5 parallel uploads: 154.93ms

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn test_parallel_uploads() {
        use std::env;
        use std::time::Instant;

        // Read parallelism level from env
        // Use env variable if provided, else saturate all tokio threads by default (num_cpus::get())
        let parallel_uploads: usize = env::var("GENEVA_PARALLEL_UPLOADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(num_cpus::get);
        let ctx = test_helpers::build_test_upload_context().await;

        // --- Warm-up: do the first upload to populate the token cache ---
        println!("🔥 Performing warm-up upload...");
        let start_warmup = Instant::now();
        let _ = ctx
            .uploader
            .upload(ctx.data.clone(), &ctx.event_name, &ctx.event_version)
            .await
            .expect("Warm-up upload failed");
        println!(
            "🔥 Warm-up upload complete in {:.2?}",
            start_warmup.elapsed()
        );

        println!("🚀 Launching {parallel_uploads} parallel uploads...");

        let start_all = Instant::now();

        let mut handles = vec![];
        for i in 0..parallel_uploads {
            let uploader = ctx.uploader.clone();
            let data = ctx.data.clone();
            let event_name = ctx.event_name.to_string();
            let event_version = ctx.event_version.to_string();

            let handle = tokio::spawn(async move {
                let start = Instant::now();
                let resp = uploader
                    .upload(data, &event_name, &event_version)
                    .await
                    .unwrap_or_else(|_| panic!("Upload {} failed", i));
                let elapsed = start.elapsed();
                println!(
                    "✅ Upload {} complete in {:.2?}. Ticket: {}",
                    i, elapsed, resp.ticket
                );
                elapsed
            });

            handles.push(handle);
        }

        let durations: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|res| res.expect("Join error in upload task"))
            .collect();

        let total_time = start_all.elapsed();

        let avg_ms =
            durations.iter().map(|d| d.as_millis()).sum::<u128>() as f64 / durations.len() as f64;
        println!("📊 Average upload duration: {:.2} ms", avg_ms);

        println!(
            "⏱️ Total elapsed for {} parallel uploads: {:.2?}",
            parallel_uploads, total_time
        );
    }
}
