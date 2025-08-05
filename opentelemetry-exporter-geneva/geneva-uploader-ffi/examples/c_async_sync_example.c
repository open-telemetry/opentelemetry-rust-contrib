/*
 * C example demonstrating both async and sync Geneva FFI usage
 * Shows the difference between blocking and non-blocking uploads
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>
#include "../include/geneva_ffi.h"

// Global variables for async callback tracking
static int async_callback_count = 0;
static GenevaError async_callback_result = GENEVA_INTERNAL_ERROR;

// Callback function for async uploads
void upload_callback(GenevaError error_code, void* user_data) {
    async_callback_count++;
    async_callback_result = error_code;
    
    printf("🔔 ASYNC CALLBACK: Upload completed with result: %d\n", error_code);
    
    if (user_data != NULL) {
        int* upload_id = (int*)user_data;
        printf("   Upload ID: %d\n", *upload_id);
    }
    
    if (error_code == GENEVA_SUCCESS) {
        printf("   ✅ Upload successful!\n");
    } else {
        printf("   ❌ Upload failed with error code: %d\n", error_code);
    }
}

// Helper function to create sample protobuf-like data
uint8_t* create_sample_log_data(size_t* data_len) {
    // This is dummy data - in real usage, this would be protobuf-encoded ResourceLogs
    const char* sample_data = "sample_log_data_protobuf_encoded";
    *data_len = strlen(sample_data);
    
    uint8_t* data = malloc(*data_len);
    memcpy(data, sample_data, *data_len);
    return data;
}

int main() {
    printf("Geneva FFI Async/Sync Upload Example\n");
    printf("=====================================\n\n");

    // Create Geneva client configuration
    GenevaConfig config = {
        .endpoint = "https://test.geneva.com",
        .environment = "test",
        .account = "testaccount", 
        .namespace_name = "testns",
        .region = "testregion",
        .config_major_version = 1,
        .auth_method = GENEVA_AUTH_MANAGED_IDENTITY,
        .tenant = "testtenant",
        .role_name = "testrole",
        .role_instance = "testinstance",
        .max_concurrent_uploads = 4,
        .cert_path = NULL,
        .cert_password = NULL
    };

    printf("📝 Creating Geneva client...\n");
    GenevaClientHandle* client = geneva_client_new(&config);
    
    if (client == NULL) {
        printf("❌ Failed to create Geneva client\n");
        const char* error = geneva_get_last_error();
        if (error != NULL) {
            printf("   Error: %s\n", error);
        }
        printf("   (This is expected in test environment without real Geneva endpoint)\n");
        return 1;
    }
    
    printf("✅ Geneva client created successfully\n\n");

    // Prepare sample data
    size_t data_len;
    uint8_t* sample_data = create_sample_log_data(&data_len);
    
    printf("📦 Sample data prepared: %zu bytes\n\n", data_len);

    // =================================================================
    // EXAMPLE 1: SYNCHRONOUS UPLOAD (BLOCKING)
    // =================================================================
    printf("🔄 EXAMPLE 1: SYNCHRONOUS UPLOAD\n");
    printf("=================================\n");
    printf("This will block until the upload completes...\n");
    
    time_t start_time = time(NULL);
    
    GenevaError sync_result = geneva_upload_logs_sync(client, sample_data, data_len);
    
    time_t end_time = time(NULL);
    double elapsed = difftime(end_time, start_time);
    
    printf("⏱️  Sync upload completed in %.2f seconds\n", elapsed);
    
    if (sync_result == GENEVA_SUCCESS) {
        printf("✅ Sync upload successful!\n");
    } else {
        printf("❌ Sync upload failed with error: %d\n", sync_result);
        if (sync_result == GENEVA_INVALID_DATA) {
            printf("   (This is expected - sample data is not valid protobuf)\n");
        }
    }
    printf("\n");

    // =================================================================
    // EXAMPLE 2: ASYNCHRONOUS UPLOAD (NON-BLOCKING)
    // =================================================================
    printf("🚀 EXAMPLE 2: ASYNCHRONOUS UPLOAD\n");
    printf("==================================\n");
    printf("This will return immediately and call callback when done...\n");
    
    // Reset callback tracking
    async_callback_count = 0;
    async_callback_result = GENEVA_INTERNAL_ERROR;
    
    int upload_id = 12345;  // User data to pass to callback
    
    start_time = time(NULL);
    
    GenevaError async_result = geneva_upload_logs(
        client, 
        sample_data, 
        data_len,
        upload_callback,
        &upload_id
    );
    
    end_time = time(NULL);
    elapsed = difftime(end_time, start_time);
    
    printf("⚡ Async upload queued in %.2f seconds\n", elapsed);
    
    if (async_result == GENEVA_ASYNC_OPERATION_PENDING) {
        printf("✅ Upload queued successfully - waiting for callback...\n");
        
        // Wait for callback (with timeout)
        int wait_count = 0;
        while (async_callback_count == 0 && wait_count < 50) {  // 5 second timeout
            usleep(100000);  // Sleep 100ms
            wait_count++;
            printf(".");
            fflush(stdout);
        }
        printf("\n");
        
        if (async_callback_count > 0) {
            printf("✅ Callback received after ~%.1f seconds\n", wait_count * 0.1);
        } else {
            printf("⏰ Timeout waiting for callback (this is expected in test environment)\n");
        }
        
    } else {
        printf("❌ Async upload failed immediately with error: %d\n", async_result);
        if (async_result == GENEVA_INVALID_DATA) {
            printf("   (This is expected - sample data is not valid protobuf)\n");
        }
    }
    printf("\n");

    // =================================================================
    // EXAMPLE 3: MULTIPLE CONCURRENT ASYNC UPLOADS
    // =================================================================
    printf("🔥 EXAMPLE 3: MULTIPLE CONCURRENT ASYNC UPLOADS\n");
    printf("===============================================\n");
    printf("Demonstrating concurrent async uploads...\n");
    
    async_callback_count = 0;
    
    int num_uploads = 3;
    int upload_ids[3] = {1001, 1002, 1003};
    
    start_time = time(NULL);
    
    for (int i = 0; i < num_uploads; i++) {
        GenevaError result = geneva_upload_logs(
            client,
            sample_data,
            data_len,
            upload_callback,
            &upload_ids[i]
        );
        
        printf("📤 Upload %d queued: %s\n", 
               upload_ids[i], 
               result == GENEVA_ASYNC_OPERATION_PENDING ? "SUCCESS" : "FAILED");
    }
    
    end_time = time(NULL);
    elapsed = difftime(end_time, start_time);
    
    printf("⚡ All %d uploads queued in %.2f seconds\n", num_uploads, elapsed);
    printf("Waiting for callbacks...\n");
    
    // Wait for all callbacks
    int wait_count = 0;
    while (async_callback_count < num_uploads && wait_count < 100) {  // 10 second timeout
        usleep(100000);  // Sleep 100ms
        wait_count++;
        if (wait_count % 10 == 0) {
            printf("📊 Callbacks received: %d/%d\n", async_callback_count, num_uploads);
        }
    }
    
    printf("✅ Completed with %d/%d callbacks received\n", async_callback_count, num_uploads);
    printf("\n");

    // =================================================================
    // WHERE DO UPLOADS GO?
    // =================================================================
    printf("🌐 WHERE DO UPLOADS ACTUALLY GO?\n");
    printf("=================================\n");
    printf("The uploads flow through this path:\n");
    printf("1. 📱 Your Application (C/Go)\n");
    printf("   ↓ calls geneva_upload_logs() or geneva_upload_logs_sync()\n");
    printf("2. 🔗 Rust FFI Layer (geneva-uploader-ffi)\n");
    printf("   ↓ converts to Rust types and calls\n");
    printf("3. 🦀 Geneva Rust Client (geneva-uploader)\n");
    printf("   ↓ handles authentication, serialization, HTTP\n");
    printf("4. 🌍 Geneva Service Endpoint\n");
    printf("   • Configured endpoint: %s\n", config.endpoint);
    printf("   • Environment: %s\n", config.environment);
    printf("   • Account: %s\n", config.account);
    printf("   • Namespace: %s\n", config.namespace_name);
    printf("   • Region: %s\n", config.region);
    printf("\n");
    printf("🔐 Authentication Method: %s\n", 
           config.auth_method == GENEVA_AUTH_MANAGED_IDENTITY ? 
           "Managed Identity (Azure)" : "Certificate");
    printf("\n");

    // Cleanup
    free(sample_data);
    geneva_client_free(client);
    
    printf("🧹 Cleanup completed\n");
    printf("✅ Example finished!\n");
    
    return 0;
}
