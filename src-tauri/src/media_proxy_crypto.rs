use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

pub(crate) fn hex_to_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(trimmed.len() / 2);
    for index in (0..trimmed.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&trimmed[index..index + 2], 16).ok()?);
    }
    Some(bytes)
}

pub(crate) fn guess_image_content_type_from_bytes(data: &[u8]) -> &'static str {
    if data.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

pub(crate) fn decrypt_im_image_bytes(encrypted: &[u8], skey: &str) -> Option<Vec<u8>> {
    if encrypted.len() <= 28 {
        return None;
    }
    let key = hex_to_bytes(skey)?;
    if key.len() != 32 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    cipher
        .decrypt(Nonce::from_slice(&encrypted[..12]), &encrypted[12..])
        .ok()
}
