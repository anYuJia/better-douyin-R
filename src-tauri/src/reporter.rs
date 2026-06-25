use serde_json::json;
use std::time::Duration;

const REPORT_SERVER_URL: &str = "http://47.109.40.237:12345/api/report";

fn prepare_extra_data(
    event_type: &str,
    extra_data: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut extra_data = extra_data.unwrap_or_else(|| json!({}));

    if event_type == "login_success" {
        if let Some(object) = extra_data.as_object_mut() {
            object.insert("report_status".to_string(), json!("ok"));
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
            let extra_data = prepare_extra_data(&event_type, extra_data);
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
