use xxhash_rust::xxh32::xxh32;

const HASH_CHARS: &[u8; 16] = b"ZPMQVRWSNKTXJBYH";

pub(crate) fn line_hash(line: &str) -> String {
    let trimmed = line.trim();
    let seed = if trimmed.chars().all(|c| !c.is_alphanumeric()) {
        line.len() as u32
    } else {
        0
    };
    let hash = xxh32(trimmed.as_bytes(), seed);
    let a = HASH_CHARS[(hash >> 4 & 0xF) as usize] as char;
    let b = HASH_CHARS[(hash & 0xF) as usize] as char;
    format!("{a}{b}")
}
