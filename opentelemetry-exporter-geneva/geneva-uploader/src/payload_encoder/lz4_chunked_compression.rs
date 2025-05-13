use lz4_flex::block::{compress_into, get_maximum_output_size};

/// Compresses input data in 64 KiB chunks using LZ4, writing each chunk's compressed data to a single
/// pre-allocated buffer. Each chunk in the output is prefixed by a 4-byte (little-endian) length header
/// indicating the size of the compressed data that follows. This chunked format allows easy and efficient
/// decompression, as each compressed chunk can be read independently by first reading its 4-byte length,
/// then the compressed payload.
///
/// # Output Buffer Layout (Block Diagram)
/// ```text
///  |<--Chunk 1-->|<--Chunk 2-->| ... |<--Chunk N-->|
///  +----+--------+----+-------+----+-----+
///  | 04 |....    | 03 |...    | 02 |..   |
///  +----+--------+----+-------+----+-----+
///
///  Where:
///    04, 03, 02  = 4-byte little-endian u32 chunk length headers
///    ...., ..., .. = compressed LZ4 data for each chunk (length matches header)
///    N     = number of chunks = input.len().div_ceil(CHUNK_SIZE)
///
/// Example for 3 chunks (lengths 4, 3, 2 bytes):
///  +----+--------+----+-------+----+-----+
///  | 04 |....    | 03 |...    | 02 |..   |
///  +----+--------+----+-------+----+-----+
///   ^len1 data1   ^len2 data2   ^len3 data3
/// ```
///
/// # Notes
/// - This chunked format is **not required by LZ4** itself, but is a common convention to allow boundary detection.
/// - Decompression requires reading 4 bytes, then decompressing the next `len` bytes, then repeating.
/// - This function is allocation-efficient: only two buffers are allocated (one for output, one for temp).
/// # TODO
/// - Investigate if true in-place compression is possible when the input is no longer needed.
///   This might let us use the input buffer as the temporary compression buffer, potentially reducing heap allocations further,
///   but care must be taken: LZ4 does not natively support in-place compression, and the compressed size may be larger than the input.
///   If single-buffer "in-place" compression is possible (e.g., with unsafe or buffer aliasing), document or implement it here.
#[allow(dead_code)]
pub(crate) fn lz4_chunked_compression(input: &[u8]) -> Vec<u8> {
    const CHUNK_SIZE: usize = 64 * 1024;

    // Pre-allocate an output buffer large enough for the worst-case total output size:
    // Each chunk may require up to get_maximum_output_size(CHUNK_SIZE) bytes for compressed data,
    // plus 4 bytes for the length header per chunk.
    let mut output = Vec::with_capacity(
        input.len().div_ceil(CHUNK_SIZE) * (4 + get_maximum_output_size(CHUNK_SIZE)),
    );

    // Temporary buffer for compressing each chunk (reused for all chunks).
    let mut temp = vec![0u8; get_maximum_output_size(CHUNK_SIZE)];

    let mut offset = 0;
    // Process the input in 64 KiB chunks.
    while offset < input.len() {
        // Determine the end index for the current chunk.
        let end = usize::min(offset + CHUNK_SIZE, input.len());
        // Get the current chunk from input.
        let chunk = &input[offset..end];

        // Compress the chunk into the temporary buffer.
        let compressed_size = compress_into(chunk, &mut temp).expect("Compression failed");

        // Write the compressed size as a 4-byte little-endian header to the output buffer.
        let len_bytes = (compressed_size as u32).to_le_bytes();
        output.extend_from_slice(&len_bytes);

        // Append the compressed data itself.
        output.extend_from_slice(&temp[..compressed_size]);

        // Move to the next chunk.
        offset = end;
    }

    output
}

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
