use crate::result::Result;

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    Ok(zstd::encode_all(data, 3)?)
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    Ok(zstd::decode_all(data)?)
}

pub fn should_compress(data: &[u8]) -> bool {
    data.len() > 1024
}
