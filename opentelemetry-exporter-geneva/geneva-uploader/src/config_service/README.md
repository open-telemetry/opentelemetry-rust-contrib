### GenevaConfigClient Flow (Certificate-Based Authentication)

The diagram below illustrates how the `GenevaConfigClient` is initialized with a client certificate (in PKCS#12 format) and then used to fetch ingestion information from the Geneva Config Service using mutual TLS (mTLS). It includes the flow for loading the certificate, handling cached tokens, making authenticated requests, and parsing the response. The client resolves the ingestion moniker from the primary `StorageAccountKeys` entry whose `AccountGroupName` exactly and case-sensitively matches the configured logical account group. When no group is configured, selection succeeds only if GCS returns one distinct group. Physical moniker names are never used to infer the account group.

```mermaid
sequenceDiagram
    participant App as User
    participant Client as GenevaConfigClient
    participant TLS as TLS backend<br/>(native-tls or rustls)
    participant GCS as Geneva Config Service

    App->>Client: new(GenevaConfigClientConfig)
    Client->>TLS: Load PKCS#12 cert
    TLS-->>Client: configured TLS connector / ClientConfig
    Client->>Client: Build reqwest::Client with mTLS

    App->>Client: get_ingestion_info()

    alt Token in cache and not expired
        Client->>App: Return cached gateway and primary-moniker map
    else Cache miss or token expired
        Client->>Client: Build HTTP GET URL
        Client->>GCS: Send HTTPS request with mTLS\n+ Query Params & Headers
        GCS-->>Client: JSON response (200 OK or error)

        alt Response contains valid moniker
            Client->>Client: Parse gateway and validate one primary per account group
            Client->>Client: Cache new token + expiry
            Client->>App: Return new gateway and primary-moniker map
        else No valid moniker
            Client->>App: Error (MonikerNotFound)
        end
    end
