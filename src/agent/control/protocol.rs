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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt: Option<String>,
}

impl Request {
    pub(super) fn stop() -> Self {
        Self {
            version: 1,
            request_id: Uuid::new_v4().to_string(),
            method: "stop".into(),
            prompt: None,
        }
    }

    pub(super) fn steer(prompt: &str) -> Self {
        Self {
            version: 1,
            request_id: Uuid::new_v4().to_string(),
            method: "steer".to_string(),
            prompt: Some(prompt.to_string()),
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
    let has_prompt = value.get("prompt").is_some();
    let request: Request = serde_json::from_value(value).map_err(|_| invalid())?;
    if id.is_none() {
        return Err(invalid());
    }
    if request.version != 1 {
        return Err(build_rejection(
            id.as_deref(),
            "unsupported_version",
            "Unsupported control protocol version; expected 1.",
        ));
    }
    if !matches!(request.method.as_str(), "steer" | "stop") {
        return Err(build_rejection(
            id.as_deref(),
            "unknown_method",
            "Unknown control method; expected steer or stop.",
        ));
    }
    if (request.method == "steer"
        && request
            .prompt
            .as_ref()
            .is_none_or(|prompt| prompt.trim().is_empty()))
        || (request.method == "stop" && has_prompt)
    {
        return Err(invalid());
    }
    Ok(request)
}

pub(super) fn build_rejection(id: Option<&str>, code: &str, message: &str) -> Response {
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

pub(super) fn build_response(id: &str, result: Result<(), SteeringError>) -> Response {
    match result {
        Ok(()) => Response {
            version: 1,
            request_id: Some(id.to_string()),
            accepted: Some(true),
            error: None,
        },
        Err(SteeringError::Rejected { code, message }) => build_rejection(Some(id), code, &message),
        Err(SteeringError::AcceptanceUnknown { message }) => Response {
            accepted: None,
            ..build_rejection(Some(id), "acceptance_unknown", &message)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stop_is_prompt_free_and_steering_still_requires_a_nonempty_string() {
        let stop = serde_json::to_value(Request::stop()).unwrap();
        assert!(stop.get("prompt").is_none());
        let parsed = parse_request(&serde_json::to_vec(&stop).unwrap()).unwrap();
        assert_eq!(parsed.method, "stop");
        for prompt in [json!(null), json!(""), json!("unexpected"), json!(7)] {
            let mut invalid = stop.clone();
            invalid["prompt"] = prompt;
            let response = parse_request(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
            assert_eq!(
                response.request_id.as_deref(),
                Some(parsed.request_id.as_str())
            );
            assert_eq!(response.error.unwrap().code, "invalid_request");
        }
        let mut steer = stop;
        steer["method"] = json!("steer");
        for prompt in [
            None,
            Some(json!(null)),
            Some(json!("  ")),
            Some(json!(false)),
        ] {
            if let Some(prompt) = prompt {
                steer["prompt"] = prompt;
            }
            assert!(parse_request(&serde_json::to_vec(&steer).unwrap()).is_err());
        }
        steer["prompt"] = json!(" keep whitespace 雪 ");
        assert_eq!(
            parse_request(&serde_json::to_vec(&steer).unwrap())
                .unwrap()
                .prompt
                .as_deref(),
            Some(" keep whitespace 雪 ")
        );
    }
}
