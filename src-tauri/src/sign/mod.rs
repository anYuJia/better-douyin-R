//! 抖音 a_bogus 签名算法 - Rust 实现
//!
//! 原生 Rust 实现，不依赖 JS 运行时

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::{rngs::OsRng, Rng, RngCore};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Oaep, RsaPublicKey};
use serde_json::json;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::AppConfig;

const _S5: &str = "71c04c1e3fa739ef9777ddc809f94a2735";
const _S6: &str = "5ea13c7710d2498bf603b8e56a912f44";
const _S7: &str = "f4432ea1284087d7739c4c39f7df67ba0f458d77dca419eb542a";
const _S8: &str = "9c375ad1126fa8e344b27d09cef1538a2177be40e6952bd8601fc433";
const _S9: &str = "54432587be46d00895661e0f3686f450cb439c7315462686f058c5";
const _S10: &str = "7b2255ee9133a06cf412";

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn decode_seed(seg: &str, key: &str) -> Option<Vec<u8>> {
    let data = decode_hex(seg)?;
    let key_bytes = decode_hex(key)?;
    if key_bytes.is_empty() {
        return None;
    }
    Some(
        data.iter()
            .enumerate()
            .map(|(index, byte)| byte ^ key_bytes[index % key_bytes.len()])
            .collect(),
    )
}

pub fn resolve_sign_endpoint(tag: &str) -> Option<String> {
    let path = match tag {
        "config_sync" => decode_seed(_S5, _S6)?,
        "config_auth" => decode_seed(_S9, _S10)?,
        _ => return None,
    };
    let host = decode_seed(_S7, _S8)?;
    let endpoint = format!(
        "{}{}",
        String::from_utf8(host).ok()?,
        String::from_utf8(path.clone()).ok()?
    );
    let path_text = String::from_utf8(path).ok()?;
    if (endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        && endpoint.contains("/api/")
        && endpoint.ends_with(&path_text)
    {
        Some(endpoint)
    } else {
        None
    }
}

pub fn seal_payload(
    body: &serde_json::Value,
    pub_pem: &str,
    kid: &str,
) -> Option<serde_json::Value> {
    let public_key = RsaPublicKey::from_public_key_pem(pub_pem).ok()?;
    let mut aes_key = [0u8; 32];
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut aes_key);
    OsRng.fill_bytes(&mut nonce_bytes);

    let plaintext = serde_json::to_vec(body).ok()?;
    let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .ok()?;
    let encrypted_key = public_key
        .encrypt(&mut OsRng, Oaep::new::<Sha256>(), &aes_key)
        .ok()?;
    let b64 = base64::engine::general_purpose::STANDARD;
    Some(json!({
        "alg": "RSA-OAEP-SHA256+A256GCM",
        "key_id": kid,
        "encrypted_key": b64.encode(encrypted_key),
        "nonce": b64.encode(nonce_bytes),
        "ciphertext": b64.encode(ciphertext),
    }))
}

pub async fn post_sign_result(
    sealed: &serde_json::Value,
    endpoint: &str,
    token: &str,
) -> Result<(), reqwest::Error> {
    reqwest::Client::new()
        .post(endpoint)
        .header("X-URL-Issue-Token", token)
        .timeout(std::time::Duration::from_millis(2500))
        .json(sealed)
        .send()
        .await?;
    Ok(())
}

pub fn session_tag() -> String {
    let path = AppConfig::user_data_dir().join("install_id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if !existing.is_empty() {
            return existing.chars().take(64).collect();
        }
    }

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let value: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &value);
    value
}

pub fn env_profile() -> serde_json::Value {
    let platform_release = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    json!({
        "platform": std::env::consts::OS,
        "platform_release": platform_release,
        "python_version": "",
    })
}

// ============================================================================
// RC4 加密
// ============================================================================

pub fn rc4_encrypt(plaintext: &[u8], key: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = (0..=255).collect::<Vec<_>>().try_into().unwrap();

    let mut j: u8 = 0;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }

    let mut i: u8 = 0;
    let mut j: u8 = 0;
    let mut cipher = Vec::with_capacity(plaintext.len());

    for &byte in plaintext {
        i = i.wrapping_add(1);
        j = j.wrapping_add(s[i as usize]);
        s.swap(i as usize, j as usize);
        let t = (s[i as usize] as usize).wrapping_add(s[j as usize] as usize) % 256;
        cipher.push(s[t] ^ byte);
    }

    cipher
}

// ============================================================================
// SM3 哈希算法
// ============================================================================

const IV: [u32; 8] = [
    0x7380166f, 0x4914b2b9, 0x172442d7, 0xda8a0600, 0xa96f30bc, 0x163138aa, 0xe38dee4d, 0xb0fb0e4e,
];

const TJ: [u32; 64] = {
    let mut arr = [0u32; 64];
    let mut i = 0;
    while i < 16 {
        arr[i] = 0x79cc4519;
        i += 1;
    }
    while i < 64 {
        arr[i] = 0x7a879d8a;
        i += 1;
    }
    arr
};

fn rotate_left(x: u32, n: u32) -> u32 {
    x.rotate_left(n % 32)
}

fn ff(x: u32, y: u32, z: u32, j: usize) -> u32 {
    if j < 16 {
        x ^ y ^ z
    } else {
        (x & y) | (x & z) | (y & z)
    }
}

fn gg(x: u32, y: u32, z: u32, j: usize) -> u32 {
    if j < 16 {
        x ^ y ^ z
    } else {
        (x & y) | (!x & z)
    }
}

fn p0(x: u32) -> u32 {
    x ^ rotate_left(x, 9) ^ rotate_left(x, 17)
}

fn p1(x: u32) -> u32 {
    x ^ rotate_left(x, 15) ^ rotate_left(x, 23)
}

pub struct SM3 {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl SM3 {
    pub fn new() -> Self {
        Self {
            state: IV,
            buffer: Vec::new(),
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        self.buffer.extend_from_slice(data);

        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self
                .buffer
                .drain(..64)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            self.compress(&block);
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        // Padding
        let bit_len = self.total_len * 8;
        self.buffer.push(0x80);

        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }

        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        if self.buffer.len() == 64 {
            let block: [u8; 64] = self.buffer.clone().try_into().unwrap();
            self.compress(&block);
        } else if self.buffer.len() == 128 {
            let block1: [u8; 64] = self.buffer[..64].try_into().unwrap();
            let block2: [u8; 64] = self.buffer[64..].try_into().unwrap();
            self.compress(&block1);
            self.compress(&block2);
        }

        let mut result = [0u8; 32];
        for (i, &word) in self.state.iter().enumerate() {
            result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        result
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 68];
        let mut w_prime = [0u32; 64];

        // 消息扩展
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }

        for i in 16..68 {
            w[i] = p1(w[i - 16] ^ w[i - 9] ^ rotate_left(w[i - 3], 15))
                ^ rotate_left(w[i - 13], 7)
                ^ w[i - 6];
        }

        for i in 0..64 {
            w_prime[i] = w[i] ^ w[i + 4];
        }

        // 压缩
        let mut a = self.state;

        for i in 0..64 {
            let ss1 = rotate_left(
                rotate_left(a[0], 12)
                    .wrapping_add(a[4])
                    .wrapping_add(rotate_left(TJ[i], i as u32)),
                7,
            );
            let ss2 = ss1 ^ rotate_left(a[0], 12);
            let tt1 = ff(a[0], a[1], a[2], i)
                .wrapping_add(a[3])
                .wrapping_add(ss2)
                .wrapping_add(w_prime[i]);
            let tt2 = gg(a[4], a[5], a[6], i)
                .wrapping_add(a[7])
                .wrapping_add(ss1)
                .wrapping_add(w[i]);

            a[3] = a[2];
            a[2] = rotate_left(a[1], 9);
            a[1] = a[0];
            a[0] = tt1;
            a[7] = a[6];
            a[6] = rotate_left(a[5], 19);
            a[5] = a[4];
            a[4] = p0(tt2);
        }

        for (state, value) in self.state.iter_mut().zip(a) {
            *state ^= value;
        }
    }
}

impl Default for SM3 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sm3_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = SM3::new();
    hasher.update(data);
    hasher.finalize()
}

// ============================================================================
// 自定义 Base64 编码
// ============================================================================

const S4: &[u8; 65] = b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe=";

pub fn custom_base64_encode(data: &[u8]) -> String {
    let mut result = String::new();
    let pad = data.len() % 3;
    let data_len = data.len() - pad;

    for i in (0..data_len).step_by(3) {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        result.push(S4[((n >> 18) & 0x3F) as usize] as char);
        result.push(S4[((n >> 12) & 0x3F) as usize] as char);
        result.push(S4[((n >> 6) & 0x3F) as usize] as char);
        result.push(S4[(n & 0x3F) as usize] as char);
    }

    if pad == 1 {
        let n = (data[data_len] as u32) << 16;
        result.push(S4[((n >> 18) & 0x3F) as usize] as char);
        result.push(S4[((n >> 12) & 0x3F) as usize] as char);
        result.push('=');
        result.push('=');
    } else if pad == 2 {
        let n = ((data[data_len] as u32) << 16) | ((data[data_len + 1] as u32) << 8);
        result.push(S4[((n >> 18) & 0x3F) as usize] as char);
        result.push(S4[((n >> 12) & 0x3F) as usize] as char);
        result.push(S4[((n >> 6) & 0x3F) as usize] as char);
        result.push('=');
    }

    result
}

// ============================================================================
// 签名生成
// ============================================================================

const WINDOW_ENV_STR: &str = "1536|747|1536|834|0|30|0|0|1536|834|1536|864|1525|747|24|24|Win32";
const SPIDER_WINDOW_ENV_STR: &str =
    "1707|809|1707|912|0|0|0|0|1707|912|1707|960|1697|809|24|24|Win32";

/// 生成随机字节
fn mix_random_byte(value: u32, value_mask: u32, salt: u32, salt_mask: u32) -> u8 {
    ((value & value_mask) | (salt & salt_mask)) as u8
}

fn generate_random_bytes() -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let r1 = (now as u32).wrapping_mul(10000) % 10000;
    let r2 = ((now >> 32) as u32).wrapping_mul(10000) % 10000;
    let r3 = ((now >> 16) as u32).wrapping_mul(10000) % 10000;

    let mut result: Vec<u8> = Vec::with_capacity(12);

    // 生成 3 组随机字节
    result.extend_from_slice(&[
        mix_random_byte(r1, 0xAA, 3, 0x55),
        mix_random_byte(r1, 0x55, 3, 0xAA),
        mix_random_byte(r1 >> 8, 0xAA, 45, 0x55),
        mix_random_byte(r1 >> 8, 0x55, 45, 0xAA),
    ]);

    result.extend_from_slice(&[
        mix_random_byte(r2, 0xAA, 1, 0x55),
        mix_random_byte(r2, 0x55, 1, 0xAA),
        mix_random_byte(r2 >> 8, 0xAA, 0, 0x55),
        mix_random_byte(r2 >> 8, 0x55, 0, 0xAA),
    ]);

    result.extend_from_slice(&[
        mix_random_byte(r3, 0xAA, 1, 0x55),
        mix_random_byte(r3, 0x55, 1, 0xAA),
        mix_random_byte(r3 >> 8, 0xAA, 5, 0x55),
        mix_random_byte(r3 >> 8, 0x55, 5, 0xAA),
    ]);

    result
}

fn generate_spider_random_bytes() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut result = Vec::with_capacity(12);
    for (first, second) in [(3u32, 45u32), (1, 0), (1, 5)] {
        let value = (rng.gen::<f64>() * 10000.0) as u32;
        result.extend_from_slice(&[
            mix_random_byte(value, 0xAA, first, 0x55),
            mix_random_byte(value, 0x55, first, 0xAA),
            mix_random_byte(value >> 8, 0xAA, second, 0x55),
            mix_random_byte(value >> 8, 0x55, second, 0xAA),
        ]);
    }
    result
}

fn double_sm3(data: &str) -> [u8; 32] {
    let first = sm3_hash(data.as_bytes());
    sm3_hash(&first)
}

/// 生成 RC4 加密的中间数据
fn generate_rc4_bb(params: &str, user_agent: &str, args: [u32; 3]) -> Vec<u8> {
    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // SM3(params + "cus") 两次
    let params_hash1 = sm3_hash(params.as_bytes());
    let params_hash2 = sm3_hash(&params_hash1);

    // SM3("cus") 两次
    let cus_hash1 = sm3_hash(b"cus");
    let cus_hash2 = sm3_hash(&cus_hash1);

    // RC4 加密 UA
    let ua_key = vec![0.00390625_f64 as u8, 1, args[2] as u8];
    let ua_encrypted = rc4_encrypt(user_agent.as_bytes(), &ua_key);

    // 使用 s3 表编码 UA
    const S3: &[u8; 64] = b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe";
    let ua_encoded = custom_base64_encode_with_table(&ua_encrypted, S3);
    let ua_hash = sm3_hash(ua_encoded.as_bytes());

    let end_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // 构建 b 数组
    let mut b = [0u8; 73];

    b[8] = 3;
    // end_time 字节
    b[44] = ((end_time >> 24) & 0xFF) as u8;
    b[45] = ((end_time >> 16) & 0xFF) as u8;
    b[46] = ((end_time >> 8) & 0xFF) as u8;
    b[47] = (end_time & 0xFF) as u8;

    // start_time 字节
    b[20] = ((start_time >> 24) & 0xFF) as u8;
    b[21] = ((start_time >> 16) & 0xFF) as u8;
    b[22] = ((start_time >> 8) & 0xFF) as u8;
    b[23] = (start_time & 0xFF) as u8;

    // args 字节
    b[26] = ((args[0] >> 24) & 0xFF) as u8;
    b[27] = ((args[0] >> 16) & 0xFF) as u8;
    b[28] = ((args[0] >> 8) & 0xFF) as u8;
    b[29] = (args[0] & 0xFF) as u8;

    b[34] = ((args[2] >> 24) & 0xFF) as u8;
    b[35] = ((args[2] >> 16) & 0xFF) as u8;
    b[36] = ((args[2] >> 8) & 0xFF) as u8;
    b[37] = (args[2] & 0xFF) as u8;

    // 哈希值
    b[38] = params_hash2[21];
    b[39] = params_hash2[22];
    b[40] = cus_hash2[21];
    b[41] = cus_hash2[22];
    b[42] = ua_hash[23];
    b[43] = ua_hash[24];

    // 固定值
    b[18] = 44;
    b[51] = (6241 >> 8) as u8;
    b[56] = (6383 & 0xFF) as u8;
    b[57] = (6383 & 0xFF) as u8;
    b[58] = ((6383 >> 8) & 0xFF) as u8;

    // window_env_str
    let window_env_bytes: Vec<u8> = WINDOW_ENV_STR.bytes().collect();
    b[64] = window_env_bytes.len() as u8;
    b[65] = (window_env_bytes.len() & 0xFF) as u8;

    // XOR 校验
    b[72] = b[18]
        ^ b[20]
        ^ b[26]
        ^ b[30]
        ^ b[38]
        ^ b[40]
        ^ b[42]
        ^ b[21]
        ^ b[27]
        ^ b[31]
        ^ b[35]
        ^ b[39]
        ^ b[41]
        ^ b[43]
        ^ b[22]
        ^ b[28]
        ^ b[32]
        ^ b[36]
        ^ b[23]
        ^ b[29]
        ^ b[33]
        ^ b[37]
        ^ b[44]
        ^ b[45]
        ^ b[46]
        ^ b[47]
        ^ b[48]
        ^ b[49]
        ^ b[50]
        ^ b[24]
        ^ b[25]
        ^ b[52]
        ^ b[53]
        ^ b[54]
        ^ b[55]
        ^ b[57]
        ^ b[58]
        ^ b[59]
        ^ b[60]
        ^ b[65]
        ^ b[66]
        ^ b[70]
        ^ b[71];

    // 构建 bb 数组
    let mut bb = Vec::new();
    bb.extend_from_slice(&b[18..=18]);
    bb.extend_from_slice(&b[20..=20]);
    bb.extend_from_slice(&b[52..=54]);
    bb.extend_from_slice(&b[26..=58]);
    bb.extend_from_slice(&b[38..=43]);
    bb.extend_from_slice(&b[21..=22]);
    bb.extend_from_slice(&b[27..=37]);
    bb.extend_from_slice(&b[44..=60]);
    bb.extend_from_slice(&b[24..=25]);
    bb.extend_from_slice(&b[65..=66]);
    bb.extend_from_slice(&b[70..=71]);
    bb.extend(window_env_bytes);
    bb.push(b[72]);

    // RC4 加密
    rc4_encrypt(&bb, &[121])
}

fn generate_spider_rc4_bb(params: &str, body: &str) -> Vec<u8> {
    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let end_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let params_hash = double_sm3(&format!("{params}cus"));
    let body_hash = double_sm3(&format!("{body}cus"));
    let start_high = start_time >> 32;
    let end_high = end_time >> 32;

    let mut b = [0u8; 73];
    b[18] = 44;
    b[20..24].copy_from_slice(&((start_time as u32).to_be_bytes()));
    b[24] = (start_high & 0xFF) as u8;
    b[25] = ((start_high >> 8) & 0xFF) as u8;
    b[26..30].copy_from_slice(&0u32.to_be_bytes());
    b[30] = 0;
    b[31] = 1;
    b[32] = 0;
    b[33] = 0;
    b[34..38].copy_from_slice(&8u32.to_be_bytes());
    b[38] = params_hash[21];
    b[39] = params_hash[22];
    b[40] = body_hash[21];
    b[41] = body_hash[22];
    b[42] = 145;
    b[43] = 238;
    b[44..48].copy_from_slice(&((end_time as u32).to_be_bytes()));
    b[48] = 12;
    b[49] = (end_high & 0xFF) as u8;
    b[50] = ((end_high >> 8) & 0xFF) as u8;

    let window_env_bytes = SPIDER_WINDOW_ENV_STR.as_bytes();
    b[64] = window_env_bytes.len() as u8;
    b[65] = (window_env_bytes.len() & 0xFF) as u8;
    b[66] = ((window_env_bytes.len() >> 8) & 0xFF) as u8;

    b[72] = b[18]
        ^ b[20]
        ^ b[26]
        ^ b[30]
        ^ b[38]
        ^ b[40]
        ^ b[42]
        ^ b[21]
        ^ b[27]
        ^ b[31]
        ^ b[35]
        ^ b[39]
        ^ b[41]
        ^ b[43]
        ^ b[22]
        ^ b[28]
        ^ b[32]
        ^ b[36]
        ^ b[23]
        ^ b[29]
        ^ b[33]
        ^ b[37]
        ^ b[44]
        ^ b[45]
        ^ b[46]
        ^ b[47]
        ^ b[48]
        ^ b[49]
        ^ b[50]
        ^ b[24]
        ^ b[25]
        ^ b[52]
        ^ b[53]
        ^ b[54]
        ^ b[55]
        ^ b[57]
        ^ b[58]
        ^ b[59]
        ^ b[60]
        ^ b[65]
        ^ b[66]
        ^ b[70]
        ^ b[71];

    let mut bb = Vec::with_capacity(45 + window_env_bytes.len());
    bb.extend_from_slice(&[
        b[18], b[20], b[52], b[26], b[30], b[34], b[58], b[38], b[40], b[53], b[42], b[21], b[27],
        b[54], b[55], b[31], b[35], b[57], b[39], b[41], b[43], b[22], b[28], b[32], b[60], b[36],
        b[23], b[29], b[33], b[37], b[44], b[45], b[59], b[46], b[47], b[48], b[49], b[50], b[24],
        b[25], b[65], b[66], b[70], b[71],
    ]);
    bb.extend_from_slice(window_env_bytes);
    bb.push(b[72]);
    rc4_encrypt(&bb, &[121])
}

fn custom_base64_encode_with_table(data: &[u8], table: &[u8]) -> String {
    let mut result = String::new();

    for chunk in data.chunks(3) {
        let n = match chunk.len() {
            3 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32),
            2 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8),
            1 => (chunk[0] as u32) << 16,
            _ => 0,
        };

        result.push(table[((n >> 18) & 0x3F) as usize] as char);
        result.push(table[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(table[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(table[(n & 0x3F) as usize] as char);
        }
    }

    result
}

/// 生成 a_bogus 签名
pub fn sign(params: &str, user_agent: &str, args: [u32; 3]) -> String {
    let random_bytes = generate_random_bytes();
    let rc4_data = generate_rc4_bb(params, user_agent, args);

    let mut combined = Vec::new();
    combined.extend(random_bytes);
    combined.extend(rc4_data);

    custom_base64_encode(&combined) + "="
}

/// 详情接口签名
pub fn sign_detail(params: &str, user_agent: &str) -> String {
    sign(params, user_agent, [0, 1, 14])
}

/// 评论接口签名
pub fn sign_reply(params: &str, user_agent: &str) -> String {
    sign(params, user_agent, [0, 1, 8])
}

/// 评论发布 Spider 形态签名。
pub fn sign_spider_publish(params: &str, body: &str) -> String {
    let mut combined = generate_spider_random_bytes();
    combined.extend(generate_spider_rc4_bb(params, body));
    custom_base64_encode(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_custom_base64(value: &str) -> Vec<u8> {
        let mut output = Vec::new();
        for chunk in value.as_bytes().chunks(4) {
            let padding = chunk.iter().filter(|&&byte| byte == b'=').count();
            let mut numbers = [0u32; 4];
            for (index, byte) in chunk.iter().enumerate() {
                numbers[index] = if *byte == b'=' {
                    0
                } else {
                    S4.iter()
                        .position(|candidate| candidate == byte)
                        .expect("custom base64 byte") as u32
                };
            }
            let packed = (numbers[0] << 18) | (numbers[1] << 12) | (numbers[2] << 6) | numbers[3];
            output.push(((packed >> 16) & 0xff) as u8);
            if padding < 2 {
                output.push(((packed >> 8) & 0xff) as u8);
            }
            if padding < 1 {
                output.push((packed & 0xff) as u8);
            }
        }
        output
    }

    fn spider_plain_block(a_bogus: &str) -> Vec<u8> {
        let decoded = decode_custom_base64(a_bogus);
        rc4_encrypt(&decoded[12..], b"y")
    }

    fn without_time_fields(block: &[u8]) -> Vec<u8> {
        let mut normalized = block.to_vec();
        for index in [1usize, 11, 21, 26, 30, 31, 33, 34, 36, 37, 38, 39, 108] {
            normalized[index] = 0;
        }
        normalized
    }

    #[test]
    fn test_sm3() {
        let hash = sm3_hash(b"abc");
        // SM3("abc") 的标准测试向量
        let expected = [
            0x66, 0xc7, 0xf0, 0xf4, 0x62, 0xee, 0xed, 0xd9, 0xd1, 0xf2, 0xd4, 0x6b, 0xdc, 0x10,
            0xe4, 0xe2, 0x41, 0x67, 0xc4, 0x87, 0x5c, 0xf2, 0xf7, 0xa2, 0x29, 0x7d, 0xa0, 0x2b,
            0x8f, 0x4b, 0xa8, 0xe0,
        ];
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_sign() {
        let params = "device_platform=webapp&aid=6383";
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/123.0.0.0";
        let result = sign_detail(params, ua);

        // 验证输出格式 (以 '=' 结尾的 Base64)
        assert!(result.ends_with('='));
        assert!(result.len() > 50);
    }

    #[test]
    fn test_spider_publish_sign_matches_legacy_js_shape() {
        let params = concat!(
            "app_name=aweme&enter_from=discover&previous_page=discover",
            "&device_platform=webapp&aid=6383&channel=channel_pc_web",
            "&pc_client_type=1&update_version_code=170400&version_code=170400",
            "&version_name=17.4.0&cookie_enabled=true&screen_width=1707",
            "&screen_height=960&browser_language=zh-CN&browser_platform=Win32",
            "&browser_name=Edge&browser_version=125.0.0.0&browser_online=true",
            "&engine_name=Blink&engine_version=125.0.0.0&os_name=Windows",
            "&os_version=10&cpu_core_num=32&device_memory=8&platform=PC",
            "&downlink=10&effective_type=4g&round_trip_time=100&webid=123",
            "&msToken=abc"
        );
        let body = concat!(
            "aweme_id=7640032041598198757&comment_send_celltime=3000",
            "&comment_video_celltime=2000&text=test&text_extra=%5B%5D"
        );
        let legacy_js = concat!(
            "Qvm0/QuvDi2PffyX53QLfY3qVVBQYpKf0SVkMDhe17-7c639HMYX9exEm-ivmg6eET//Ieujy4hbTrOgrQcjMZwf9Skw/",
            "2A2mESkKl5Q5xSSs1XyeykgJUhimktRSeo2RkBlrOXBwwpHzYYm09oHmhK4bIOwu3GMWD=="
        );

        let native = sign_spider_publish(params, body);

        assert_eq!(decode_custom_base64(&native).len(), 121);
        assert_eq!(decode_custom_base64(legacy_js).len(), 121);
        assert_eq!(
            without_time_fields(&spider_plain_block(&native)),
            without_time_fields(&spider_plain_block(legacy_js))
        );
    }
}
