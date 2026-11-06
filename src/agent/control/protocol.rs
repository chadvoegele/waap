use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub(super) const PROTOCOL_VERSION: u64 = 1;

use super::super::commands::CommandError;
use super::super::commands::{CommandResult, Operation};

#[derive(Debug, Serialize)]
pub(super) struct Request {
    pub(super) version: u64,
    pub(super) request_id: String,
    #[serde(flatten)]
    pub(super) operation: Operation,
}

impl Request {
    pub(super) fn stop() -> Self {
        Self::new(Operation::Stop {})
    }

    pub(super) fn steer(prompt: &str) -> Self {
        Self::new(Operation::Steer {
            prompt: prompt.into(),
        })
    }

    fn new(operation: Operation) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation,
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
        .map_err(|_| build_rejection(None, "invalid_request", "Expected a UTF-8 JSON request."))?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .map(str::to_owned);
    let invalid = || {
        build_rejection(
            id.as_deref(),
            "invalid_request",
            "Expected version, UUID request_id, and method; steer requires a nonempty prompt and stop forbids prompt.",
        )
    };
    let mut fields = value.as_object().cloned().ok_or_else(invalid)?;
    let version = fields
        .remove("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(invalid)?;
    fields.remove("request_id");
    let request_id = id.clone().ok_or_else(invalid)?;
    let method = fields
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if version != PROTOCOL_VERSION {
        return Err(build_rejection(
            Some(&request_id),
            "unsupported_version",
            "Unsupported control protocol version; expected 1.",
        ));
    }
    if !matches!(method, "steer" | "stop") {
        return Err(build_rejection(
            Some(&request_id),
            "unknown_method",
            "Unknown control method; expected steer or stop.",
        ));
    }
    let operation: Operation =
        serde_json::from_value(Value::Object(fields)).map_err(|_| invalid())?;
    if matches!(&operation, Operation::Steer { prompt } if prompt.trim().is_empty()) {
        return Err(invalid());
    }
    let request = Request {
        version,
        request_id,
        operation,
    };
    Ok(request)
}

pub(super) fn build_rejection(id: Option<&str>, code: &str, message: &str) -> Response {
    Response {
        version: PROTOCOL_VERSION,
        request_id: id.map(str::to_owned),
        accepted: Some(false),
        error: Some(ResponseError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}

pub(super) fn build_response(id: &str, result: CommandResult) -> Response {
    match result {
        Ok(()) => Response {
            version: PROTOCOL_VERSION,
            request_id: Some(id.to_string()),
            accepted: Some(true),
            error: None,
        },
        Err(CommandError::Rejected { code, message }) => build_rejection(Some(id), code, &message),
        Err(CommandError::AcceptanceUnknown { message }) => Response {
            accepted: None,
            ..build_rejection(Some(id), "acceptance_unknown", &message)
        },
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
