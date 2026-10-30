use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::super::backend::SteeringError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) version: u64,
    pub(super) request_id: String,
    pub(super) method: String,
    pub(super) prompt: String,
}

impl Request {
    pub(super) fn steer(prompt: &str) -> Self {
        Self {
            version: 1,
            request_id: Uuid::new_v4().to_string(),
            method: "steer".to_string(),
            prompt: prompt.to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Response {
    pub(super) version: u64,
    pub(super) request_id: Option<String>,
    // Required on the wire, even though its value may be null.
    #[serde(deserialize_with = "Option::deserialize")]
    pub(super) accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<ResponseError>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseError {
    pub(super) code: String,
    pub(super) message: String,
}

pub(super) fn parse_request(frame: &[u8]) -> Result<Request, Response> {
    let value: Value = serde_json::from_slice(frame)
        .map_err(|_| rejection(None, "invalid_request", "Expected a UTF-8 JSON request."))?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .map(str::to_owned);
    let invalid = || {
        rejection(
            id.as_deref(),
            "invalid_request",
            "Expected version, UUID request_id, method, and a nonempty prompt.",
        )
    };
    let request: Request = serde_json::from_value(value).map_err(|_| invalid())?;
    if id.is_none() || request.prompt.trim().is_empty() {
        return Err(invalid());
    }
    if request.version != 1 {
        return Err(rejection(
            id.as_deref(),
            "unsupported_version",
            "Unsupported control protocol version; expected 1.",
        ));
    }
    if request.method != "steer" {
        return Err(rejection(
            id.as_deref(),
            "unknown_method",
            "Unknown control method; expected steer.",
        ));
    }
    Ok(request)
}

pub(super) fn rejection(id: Option<&str>, code: &str, message: &str) -> Response {
    Response {
        version: 1,
        request_id: id.map(str::to_owned),
        accepted: Some(false),
        error: Some(ResponseError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}

pub(super) fn response(id: &str, result: Result<(), SteeringError>) -> Response {
    match result {
        Ok(()) => Response {
            version: 1,
            request_id: Some(id.to_string()),
            accepted: Some(true),
            error: None,
        },
        Err(SteeringError::Rejected { code, message }) => rejection(Some(id), code, &message),
        Err(SteeringError::AcceptanceUnknown { message }) => Response {
            accepted: None,
            ..rejection(Some(id), "acceptance_unknown", &message)
        },
    }
}
