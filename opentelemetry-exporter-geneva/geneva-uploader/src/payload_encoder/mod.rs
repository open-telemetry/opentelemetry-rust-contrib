mod lz4_chunked_compression;

#[cfg(test)]
mod tests {
    use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
    use lz4_flex::block::decompress;

    #[test]
    fn test_roundtrip_large_input() {
        // Very large input (10 MB of repeating pattern)
        let input = vec![0xAB; 10 * 1024 * 1024];
        let compressed = lz4_chunked_compression(&input);
        let decompressed = decompress_chunked_lz4(&compressed, input.len());
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_roundtrip_empty_input() {
        // Empty input
        let input: Vec<u8> = Vec::new();
        let compressed = lz4_chunked_compression(&input);
        let decompressed = decompress_chunked_lz4(&compressed, input.len());
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_roundtrip_unicode_input() {
        // Input with valid UTF-8 (e.g. emoji, accented chars in multiple languages)
        let s = "😀 Grüße aus München! こんにちは世界 🌏 안녕하세요 세계";
        let input = s.as_bytes();
        let compressed = lz4_chunked_compression(input);
        let decompressed = decompress_chunked_lz4(&compressed, input.len());
        assert_eq!(decompressed, input);
        assert_eq!(std::str::from_utf8(&decompressed).unwrap(), s);
    }

    #[test]
    fn test_roundtrip_exact_chunk_size() {
        // Input exactly one chunk
        const CHUNK_SIZE: usize = 64 * 1024;
        let input = vec![0xCD; CHUNK_SIZE];
        let compressed = lz4_chunked_compression(&input);
        let decompressed = decompress_chunked_lz4(&compressed, input.len());
        assert_eq!(decompressed, input);
    }

    // Helper function to decompress chunked output
    // Each chunk: [4 bytes little-endian compressed_len][compressed data...]
    fn decompress_chunked_lz4(compressed: &[u8], total_uncompressed_len: usize) -> Vec<u8> {
        const CHUNK_SIZE: usize = 64 * 1024;
        let mut offset = 0;
        let mut out = Vec::with_capacity(total_uncompressed_len);
        let mut bytes_remaining = total_uncompressed_len;
        while offset < compressed.len() {
            // Read compressed chunk length
            let compressed_len =
                u32::from_le_bytes(compressed[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            // Read compressed chunk
            let chunk = &compressed[offset..offset + compressed_len];

            // Determine uncompressed size for this chunk
            let expected_uncompressed = if bytes_remaining >= CHUNK_SIZE {
                CHUNK_SIZE
            } else {
                bytes_remaining
            };

            let decompressed_chunk = decompress(chunk, expected_uncompressed).unwrap();
            out.extend_from_slice(&decompressed_chunk);
            offset += compressed_len;
            bytes_remaining -= decompressed_chunk.len();
        }
        out
    }
}
