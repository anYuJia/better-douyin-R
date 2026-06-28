pub(crate) fn extract_payload_url(value: &serde_json::Value) -> Option<String> {
    if let Some(url) = value.as_str() {
        let url = url.trim();
        return (!url.is_empty()).then(|| url.to_string());
    }

    if let Some(values) = value.as_array() {
        for value in values {
            if let Some(url) = extract_payload_url(value) {
                return Some(url);
            }
        }
        return None;
    }

    for key in ["url", "play_url", "play_addr", "download_addr", "url_list"] {
        if let Some(url) = value.get(key).and_then(extract_payload_url) {
            return Some(url);
        }
    }

    None
}
