use super::*;
use serde_json::json;

#[test]
fn typed_variants_preserve_wire_shape_and_reject_extra_fields() {
    for request in [Request::stop(), Request::steer("  Unicode 雪  ")] {
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["version"], 1);
        assert_eq!(wire["request_id"], request.request_id);
        assert_eq!(
            wire["method"],
            match request.operation {
                Operation::Steer { .. } => "steer",
                Operation::Stop {} => "stop",
            }
        );
        let parsed = parse_request(&serde_json::to_vec(&wire).unwrap()).unwrap();
        assert_eq!(parsed.operation, request.operation);
        for extra in ["unexpected", "deadline", "payload"] {
            let mut invalid = wire.clone();
            invalid[extra] = json!(null);
            let response = parse_request(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
            assert_eq!(
                response.request_id.as_deref(),
                Some(request.request_id.as_str())
            );
            assert_eq!(response.error.unwrap().code, "invalid_request");
        }
        for (key, value, code) in [
            ("version", json!(2), "unsupported_version"),
            ("method", json!("future"), "unknown_method"),
            ("method", json!(false), "invalid_request"),
            ("version", json!(null), "invalid_request"),
        ] {
            let mut invalid = wire.clone();
            invalid[key] = value;
            let response = parse_request(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
            assert_eq!(
                response.request_id.as_deref(),
                Some(request.request_id.as_str())
            );
            assert_eq!(response.error.unwrap().code, code);
        }
    }
}

#[test]
fn stop_is_prompt_free_and_steering_still_requires_a_nonempty_string() {
    let stop = serde_json::to_value(Request::stop()).unwrap();
    assert!(stop.get("prompt").is_none());
    let parsed = parse_request(&serde_json::to_vec(&stop).unwrap()).unwrap();
    assert_eq!(parsed.operation, Operation::Stop {});
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
            .operation,
        Operation::Steer {
            prompt: " keep whitespace 雪 ".into()
        }
    );
}
