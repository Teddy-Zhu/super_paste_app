pub fn hash_content(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

pub fn generate_content_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
