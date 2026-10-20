use serde_json::{json, Value};
use uuid::Uuid;

use super::super::backend::SteeringError;

pub(super) const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(super) struct Request {
    pub(super) request_id: String,
    pub(super) prompt: String,
}

pub(super) fn parse_request(frame: &[u8]) -> Result<Request, Value> {
    let value: Value = serde_json::from_slice(frame)
        .map_err(|_| rejection(None, "invalid_request", "Expected a UTF-8 JSON request."))?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok());
    let invalid = || {
        rejection(
            id,
            "invalid_request",
            "Expected version, UUID request_id, method, and a nonempty prompt.",
        )
    };
    let object = value.as_object().ok_or_else(invalid)?;
    if id.is_none() || !object.contains_key("version") || !object.contains_key("method") {
        return Err(invalid());
    }
    let version = value["version"].as_u64().ok_or_else(invalid)?;
    if version != 1 {
        return Err(rejection(
            id,
            "unsupported_version",
            "Unsupported control protocol version; expected 1.",
        ));
    }
    let method = value["method"].as_str().ok_or_else(invalid)?;
    if method != "steer" {
        return Err(rejection(
            id,
            "unknown_method",
            "Unknown control method; expected steer.",
        ));
    }
    let prompt = value["prompt"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(invalid)?;
    if object.len() != 4 {
        return Err(invalid());
    }
    Ok(Request {
        request_id: id.unwrap().to_string(),
        prompt: prompt.to_string(),
    })
}

pub(super) fn rejection(id: Option<&str>, code: &str, message: &str) -> Value {
    json!({"version": 1, "request_id": id, "accepted": false,
        "error": {"code": code, "message": message}})
}

pub(super) fn response(id: &str, result: Result<(), SteeringError>) -> Value {
    match result {
        Ok(()) => json!({"version": 1, "request_id": id, "accepted": true}),
        Err(SteeringError::Rejected { code, message }) => rejection(Some(id), code, &message),
        Err(SteeringError::AcceptanceUnknown { message }) => json!({
            "version": 1, "request_id": id, "accepted": null,
            "error": {"code": "acceptance_unknown", "message": message}}),
    }
}
