### GenevaConfigClient Flow (Certificate-Based Authentication)

The diagram below illustrates how the `GenevaConfigClient` is initialized with a client certificate (in PKCS#12 format) and then used to fetch ingestion information from the Geneva Config Service using mutual TLS (mTLS). It includes the flow for loading the certificate, handling cached tokens, making authenticated requests, and parsing the response (including primary diagnostic monikers).

```mermaid
sequenceDiagram
    participant App as User
    participant Client as GenevaConfigClient
    participant TLS as native_tls
    participant GCS as Geneva Config Service

    App->>Client: new(GenevaConfigClientConfig)
    Client->>TLS: Load PKCS#12 cert
    TLS-->>Client: native_tls::TlsConnector
    Client->>Client: Build reqwest::Client with mTLS

    App->>Client: get_ingestion_info()

    alt Token in cache and not expired
        Client->>App: Return cached (IngestionGatewayInfo, MonikerInfo)
    else Cache miss or token expired
        Client->>Client: Build HTTP GET URL
        Client->>GCS: Send HTTPS request with mTLS\n+ Query Params & Headers
        GCS-->>Client: JSON response (200 OK or error)

        alt Response contains valid moniker
            Client->>Client: Parse IngestionGatewayInfo and MonikerInfo
            Client->>Client: Cache new token + expiry
            Client->>App: Return new (IngestionGatewayInfo, MonikerInfo)
        else No valid moniker
            Client->>App: Error (MonikerNotFound)
        end
    end