/*
 * C example demonstrating how to use the Geneva FFI from C code
 * This serves as both documentation and a functional test
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../include/geneva_ffi.h"

int main() {
    printf("Geneva FFI C Example\n");
    printf("====================\n\n");

    // Test 1: Create client with null config (should fail)
    printf("Test 1: Creating client with NULL config...\n");
    GenevaClientHandle* client = geneva_client_new(NULL);
    if (client == NULL) {
        printf("✓ Correctly returned NULL for NULL config\n");
    } else {
        printf("✗ Should have returned NULL for NULL config\n");
        geneva_client_free(client);
    }
    printf("\n");

    // Test 2: Create client with valid config
    printf("Test 2: Creating client with valid config...\n");
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
        .max_concurrent_uploads = -1,
        .cert_path = NULL,
        .cert_password = NULL
    };

    client = geneva_client_new(&config);
    if (client != NULL) {
        printf("✓ Client created successfully\n");
        
        // Test 3: Upload logs with invalid data (sync)
        printf("Test 3: Uploading logs with invalid data (sync)...\n");
        uint8_t dummy_data[] = {0x01, 0x02, 0x03, 0x04};
        GenevaError result = geneva_upload_logs_sync(client, dummy_data, sizeof(dummy_data));
        if (result != GENEVA_SUCCESS) {
            printf("✓ Sync upload failed as expected with invalid data (error: %d)\n", result);
        } else {
            printf("✗ Sync upload should have failed with invalid data\n");
        }
        
        geneva_client_free(client);
        printf("✓ Client freed successfully\n");
    } else {
        printf("✗ Client creation failed (expected for test environment)\n");
        const char* error = geneva_get_last_error();
        if (error != NULL) {
            printf("   Error: %s\n", error);
        }
    }
    printf("\n");

    // Test 4: Test error handling
    printf("Test 4: Testing error handling...\n");
    
    // Upload sync with null handle
    GenevaError result = geneva_upload_logs_sync(NULL, NULL, 0);
    if (result == GENEVA_INVALID_DATA) {
        printf("✓ Correctly returned INVALID_DATA for NULL handle\n");
    } else {
        printf("✗ Should have returned INVALID_DATA for NULL handle\n");
    }

    // Free null handle (should not crash)
    geneva_client_free(NULL);
    printf("✓ Free with NULL handle completed without crash\n");
    printf("\n");

    // Test 5: Certificate authentication config
    printf("Test 5: Testing certificate authentication config...\n");
    GenevaConfig cert_config = {
        .endpoint = "https://test.geneva.com",
        .environment = "test",
        .account = "testaccount",
        .namespace_name = "testns",
        .region = "testregion",
        .config_major_version = 1,
        .auth_method = GENEVA_AUTH_CERTIFICATE,
        .tenant = "testtenant",
        .role_name = "testrole",
        .role_instance = "testinstance",
        .max_concurrent_uploads = 4,
        .cert_path = "/path/to/cert.p12",
        .cert_password = "password"
    };

    client = geneva_client_new(&cert_config);
    if (client != NULL) {
        printf("✓ Certificate auth client created\n");
        geneva_client_free(client);
    } else {
        printf("✗ Certificate auth client creation failed (expected)\n");
    }

    printf("\nAll FFI interface tests completed!\n");
    return 0;
}
