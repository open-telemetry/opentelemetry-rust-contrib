use anyhow::{anyhow, bail, Result};
use lz4_flex::block::decompress_into;

const CHUNK_SIZE: usize = 64 * 1024;

pub(crate) fn decompress_chunked_lz4(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut offset = 0usize;
    let mut output = Vec::new();

    while offset < compressed.len() {
        let header = compressed
            .get(offset..offset + 4)
            .ok_or_else(|| anyhow!("truncated chunk header at offset {offset}"))?;
        let chunk_len = u32::from_le_bytes(header.try_into().expect("4-byte slice")) as usize;
        offset += 4;

        let chunk = compressed
            .get(offset..offset + chunk_len)
            .ok_or_else(|| anyhow!("truncated chunk payload at offset {offset}"))?;
        offset += chunk_len;

        let mut scratch = vec![0u8; CHUNK_SIZE];
        let written = decompress_into(chunk, &mut scratch)
            .map_err(|err| anyhow!("LZ4 decode failed: {err:?}"))?;
        output.extend_from_slice(&scratch[..written]);
    }

    if offset != compressed.len() {
        bail!("extra trailing bytes after LZ4 decode");
    }

    Ok(output)
}
