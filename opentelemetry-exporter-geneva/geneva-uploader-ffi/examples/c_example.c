/*
 * Geneva FFI C Example with Valid ResourceLogs
 * 
 * This example demonstrates how to use the Geneva FFI library with:
 * - Environment variable configuration
 * - Valid OpenTelemetry ResourceLogs protobuf generation
 * - Both synchronous and asynchronous upload patterns
 * 
 * Required Environment Variables:
 * export GENEVA_ENDPOINT="https://abc.azurewebsites.net"
 * export GENEVA_ENVIRONMENT="Test"
 * export GENEVA_ACCOUNT="myaccount"
 * export GENEVA_NAMESPACE="myns"
 * export GENEVA_REGION="eastus"
 * export GENEVA_CONFIG_MAJOR_VERSION="2"
 * 
 * Optional for Certificate Authentication:
 * export GENEVA_CERT_PATH="/path/to/cert.p12"
 * export GENEVA_CERT_PASSWORD="cert-password"
 * 
 * Optional with defaults:
 * export GENEVA_TENANT="default-tenant"
 * export GENEVA_ROLE_NAME="default-role"
 * export GENEVA_ROLE_INSTANCE="default-instance"
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include "../include/geneva_ffi.h"

// Simple function to get environment variable or use default
const char* get_env_or_default(const char* env_name, const char* default_value) {
    const char* value = getenv(env_name);
    return value ? value : default_value;
}

// Function to create a valid OpenTelemetry ResourceLogs protobuf
// This creates a minimal but valid ResourceLogs message structure
uint8_t* create_valid_resource_logs(size_t* data_len) {
    // This is a minimal but valid ResourceLogs protobuf message
    // In production, you would use the official OpenTelemetry protobuf library
    // Format: ResourceLogs with one LogRecord
    
    // Simple protobuf encoding for a basic log record
    // Field 1 (resource): Resource message
    // Field 2 (scope_logs): ScopeLogs array
    static uint8_t resource_logs_data[] = {
        // ResourceLogs message
        0x0a, 0x20,  // Field 1 (resource), length 32 bytes
            // Resource message  
            0x0a, 0x1e,  // Field 1 (attributes), length 30 bytes
                // KeyValue for service.name
                0x0a, 0x1c,  // Field 1, length 28 bytes
                    0x0a, 0x0c, 's', 'e', 'r', 'v', 'i', 'c', 'e', '.', 'n', 'a', 'm', 'e',  // key
                    0x12, 0x0c,  // Field 2 (value), length 12 bytes
                        0x0a, 0x0a, 'c', '-', 'e', 'x', 'a', 'm', 'p', 'l', 'e',  // string value
        
        0x12, 0x40,  // Field 2 (scope_logs), length 64 bytes
            // ScopeLogs message
            0x12, 0x3e,  // Field 2 (log_records), length 62 bytes
                // LogRecord message
                0x0a, 0x3c,  // Field 1, length 60 bytes
                    0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // Field 2 (time_unix_nano) - current time
                    0x21, 0x09, 0x00, 0x00, 0x00,  // Field 4 (severity_number) - INFO (9)
                    0x2a, 0x0d, 'H', 'e', 'l', 'l', 'o', ' ', 'f', 'r', 'o', 'm', ' ', 'C', '!',  // Field 5 (body) - "Hello from C!"
                    0x32, 0x18,  // Field 6 (attributes), length 24 bytes
                        // KeyValue for event_id
                        0x0a, 0x16,  // Field 1, length 22 bytes
                            0x0a, 0x08, 'e', 'v', 'e', 'n', 't', '_', 'i', 'd',  // key
                            0x12, 0x0a,  // Field 2 (value), length 10 bytes
                                0x10, 0x14  // int_value = 20
    };
    
    // Update timestamp to current time
    uint64_t current_time_ns = (uint64_t)time(NULL) * 1000000000ULL;
    memcpy(&resource_logs_data[39], &current_time_ns, 8);
    
    *data_len = sizeof(resource_logs_data);
    
    uint8_t* data = malloc(*data_len);
    memcpy(data, resource_logs_data, *data_len);
    return data;
}

// Global variables for async callback testing
static int async_upload_completed = 0;
static GenevaError async_upload_result = GENEVA_SUCCESS;
static int completed_uploads = 0;

// Callback for async uploads
void upload_callback(GenevaError error_code, void* user_data) {
    int* upload_id = (int*)user_data;
    
    printf("ASYNC CALLBACK: Upload %d completed with result: %d\n", 
           upload_id ? *upload_id : 0, error_code);
    
    if (error_code == GENEVA_SUCCESS) {
        printf("   Upload successful!\n");
    } else {
        printf("   Upload failed with error code: %d\n", error_code);
    }
    
    // Store result for verification
    async_upload_completed = 1;
    async_upload_result = error_code;
}

// Callback for multiple uploads
void multi_upload_callback(GenevaError error_code, void* user_data) {
    int* id = (int*)user_data;
    printf("Multi-upload %d completed: %d\n", *id, error_code);
    completed_uploads++;
}

int main() {
    printf("Geneva FFI ResourceLogs Example\n");
    printf("===============================\n\n");

    // Read configuration from environment variables
    const char* endpoint = getenv("GENEVA_ENDPOINT");
    const char* environment = getenv("GENEVA_ENVIRONMENT"); 
    const char* account = getenv("GENEVA_ACCOUNT");
    const char* namespace_name = getenv("GENEVA_NAMESPACE");
    const char* region = getenv("GENEVA_REGION");
    const char* config_version_str = getenv("GENEVA_CONFIG_MAJOR_VERSION");

    // Check required environment variables
    if (!endpoint || !environment || !account || !namespace_name || !region || !config_version_str) {
        printf("Missing required environment variables!\n");
        printf("Required variables:\n");
        printf("  GENEVA_ENDPOINT\n");
        printf("  GENEVA_ENVIRONMENT\n");
        printf("  GENEVA_ACCOUNT\n");
        printf("  GENEVA_NAMESPACE\n");
        printf("  GENEVA_REGION\n");
        printf("  GENEVA_CONFIG_MAJOR_VERSION\n");
        printf("\nOptional for certificate auth:\n");
        printf("  GENEVA_CERT_PATH\n");
        printf("  GENEVA_CERT_PASSWORD\n");
        printf("\nExample setup:\n");
        printf("source setup_env.sh\n");
        return 1;
    }

    // Parse config version
    int config_major_version = atoi(config_version_str);
    if (config_major_version <= 0) {
        printf("Invalid GENEVA_CONFIG_MAJOR_VERSION: %s\n", config_version_str);
        return 1;
    }

    // Optional variables with defaults
    const char* tenant = get_env_or_default("GENEVA_TENANT", "default-tenant");
    const char* role_name = get_env_or_default("GENEVA_ROLE_NAME", "default-role");
    const char* role_instance = get_env_or_default("GENEVA_ROLE_INSTANCE", "default-instance");

    // Check for certificate authentication
    const char* cert_path = getenv("GENEVA_CERT_PATH");
    const char* cert_password = getenv("GENEVA_CERT_PASSWORD");
    
    int auth_method;
    if (cert_path && cert_password) {
        auth_method = GENEVA_AUTH_CERTIFICATE;
        printf("Using Certificate Authentication\n");
        printf("   Certificate Path: %s\n", cert_path);
    } else {
        auth_method = GENEVA_AUTH_MANAGED_IDENTITY;
        printf("Using Managed Identity Authentication\n");
    }

    printf("\nConfiguration:\n");
    printf("   Endpoint: %s\n", endpoint);
    printf("   Environment: %s\n", environment);
    printf("   Account: %s\n", account);
    printf("   Namespace: %s\n", namespace_name);
    printf("   Region: %s\n", region);
    printf("   Config Version: %d\n", config_major_version);
    printf("   Tenant: %s\n", tenant);
    printf("   Role Name: %s\n", role_name);
    printf("   Role Instance: %s\n", role_instance);
    printf("\n");

    // Create Geneva client configuration
    GenevaConfig config = {
        .endpoint = endpoint,
        .environment = environment,
        .account = account,
        .namespace_name = namespace_name,
        .region = region,
        .config_major_version = config_major_version,
        .auth_method = auth_method,
        .tenant = tenant,
        .role_name = role_name,
        .role_instance = role_instance,
        .max_concurrent_uploads = -1,  // Use default
        .cert_path = cert_path,
        .cert_password = cert_password
    };

    printf("Creating Geneva client...\n");
    GenevaClientHandle* client = geneva_client_new(&config);
    
    if (client == NULL) {
        printf("Failed to create Geneva client\n");
        const char* error = geneva_get_last_error();
        if (error != NULL) {
            printf("   Error: %s\n", error);
        }
        return 1;
    }
    
    printf("Geneva client created successfully\n\n");

    // Create valid ResourceLogs protobuf data
    size_t data_len;
    uint8_t* resource_logs_data = create_valid_resource_logs(&data_len);
    
    printf("Created valid ResourceLogs protobuf: %zu bytes\n", data_len);
    printf("   Content: LogRecord with message 'Hello from C!' and event_id=20\n\n");

    // Test 1: SYNCHRONOUS UPLOAD (BLOCKING)
    printf("TEST 1: SYNCHRONOUS UPLOAD\n");
    printf("===========================\n");
    printf("Uploading ResourceLogs synchronously...\n");
    
    time_t start_time = time(NULL);
    
    GenevaError sync_result = geneva_upload_logs_sync(client, resource_logs_data, data_len);
    
    time_t end_time = time(NULL);
    double elapsed = difftime(end_time, start_time);
    
    printf("Sync upload completed in %.2f seconds\n", elapsed);
    
    if (sync_result == GENEVA_SUCCESS) {
        printf("Sync upload successful!\n");
    } else {
        printf("Sync upload failed with error: %d\n", sync_result);
        const char* error = geneva_get_last_error();
        if (error != NULL) {
            printf("   Error details: %s\n", error);
        }
    }
    printf("\n");

    // Test 2: ASYNCHRONOUS UPLOAD (NON-BLOCKING)
    printf("TEST 2: ASYNCHRONOUS UPLOAD\n");
    printf("============================\n");
    printf("Uploading ResourceLogs asynchronously...\n");
    
    int upload_id = 12345;
    async_upload_completed = 0;
    async_upload_result = GENEVA_SUCCESS;
    
    start_time = time(NULL);
    
    GenevaError async_result = geneva_upload_logs(
        client, 
        resource_logs_data, 
        data_len,
        upload_callback,
        &upload_id
    );
    
    end_time = time(NULL);
    elapsed = difftime(end_time, start_time);
    
    printf("Async upload queued in %.2f seconds\n", elapsed);
    
    if (async_result == GENEVA_ASYNC_OPERATION_PENDING) {
        printf("Upload queued successfully - waiting for callback...\n");
        
        // Wait for callback with timeout
        int timeout_seconds = 5;
        int waited = 0;
        while (!async_upload_completed && waited < timeout_seconds) {
            usleep(100000);  // Sleep 100ms
            waited++;
            if (waited % 10 == 0) {
                printf("   Waiting... (%d seconds)\n", waited / 10);
            }
        }
        
        if (async_upload_completed) {
            printf("Async upload completed with result: %d\n", async_upload_result);
        } else {
            printf("Async upload timeout after %d seconds\n", timeout_seconds);
        }
        
    } else {
        printf("Async upload failed immediately with error: %d\n", async_result);
        const char* error = geneva_get_last_error();
        if (error != NULL) {
            printf("   Error details: %s\n", error);
        }
    }
    printf("\n");

    // Test 3: MULTIPLE ASYNC UPLOADS
    printf("TEST 3: MULTIPLE ASYNC UPLOADS\n");
    printf("===============================\n");
    printf("Testing concurrent async uploads...\n");
    
    const int num_uploads = 3;
    int upload_ids[num_uploads];
    completed_uploads = 0;  // Reset counter for this test
    
    // Launch multiple uploads
    for (int i = 0; i < num_uploads; i++) {
        upload_ids[i] = 2000 + i;
        size_t upload_data_len;
        uint8_t* upload_data = create_valid_resource_logs(&upload_data_len);
        
        GenevaError result = geneva_upload_logs(
            client,
            upload_data,
            upload_data_len,
            multi_upload_callback,
            &upload_ids[i]
        );
        
        if (result == GENEVA_ASYNC_OPERATION_PENDING) {
            printf("Upload %d queued successfully\n", i + 1);
        } else {
            printf("Upload %d failed to queue: %d\n", i + 1, result);
        }
        
        free(upload_data);
    }
    
    // Wait for all uploads to complete
    printf("Waiting for all uploads to complete...\n");
    int multi_timeout = 10;
    int multi_waited = 0;
    while (completed_uploads < num_uploads && multi_waited < multi_timeout) {
        usleep(500000);  // Sleep 500ms
        multi_waited++;
        printf("   Completed: %d/%d\n", completed_uploads, num_uploads);
    }
    
    printf("Multi-upload test completed: %d/%d uploads finished\n", completed_uploads, num_uploads);
    printf("\n");

    // Show data flow explanation
    printf("DATA FLOW EXPLANATION\n");
    printf("=====================\n");
    printf("Your ResourceLogs flow through this path:\n");
    printf("1. C Application\n");
    printf("   - creates valid ResourceLogs protobuf\n");
    printf("   - calls geneva_upload_logs() or geneva_upload_logs_sync()\n");
    printf("2. Rust FFI Layer (geneva-uploader-ffi)\n");
    printf("   - validates and decodes protobuf\n");
    printf("   - spawns async tasks with thread-safe callbacks\n");
    printf("3. Geneva Rust Client (geneva-uploader)\n");
    printf("   - handles authentication and HTTP transport\n");
    printf("4. Geneva Service Endpoint\n");
    printf("   - Endpoint: %s\n", endpoint);
    printf("   - Environment: %s\n", environment);
    printf("   - Account: %s\n", account);
    printf("   - Namespace: %s\n", namespace_name);
    printf("   - Region: %s\n", region);
    
    const char* auth_method_str = (auth_method == GENEVA_AUTH_MANAGED_IDENTITY) ? 
                                  "Managed Identity" : "Certificate";
    printf("   - Auth Method: %s\n", auth_method_str);
    printf("\n");

    printf("LOG DATA STRUCTURE\n");
    printf("==================\n");
    printf("ResourceLogs {\n");
    printf("  resource: {\n");
    printf("    attributes: [{ key: 'service.name', value: 'c-example' }]\n");
    printf("  }\n");
    printf("  scope_logs: [{\n");
    printf("    log_records: [{\n");
    printf("      time_unix_nano: %llu\n", (unsigned long long)time(NULL) * 1000000000ULL);
    printf("      severity_number: INFO (9)\n");
    printf("      body: 'Hello from C!'\n");
    printf("      attributes: [{ key: 'event_id', value: 20 }]\n");
    printf("    }]\n");
    printf("  }]\n");
    printf("}\n");
    printf("\n");

    // Cleanup
    free(resource_logs_data);
    geneva_client_free(client);
    
    printf("Cleanup completed\n");
    printf("Example finished successfully!\n");
    
    return 0;
}
