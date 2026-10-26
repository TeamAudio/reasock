//! JSON-RPC 2.0 envelope handling, independent of REAPER.
//!
//! See https://www.jsonrpc.org/specification. Params are by-name only.

use serde_json::{json, Map, Value};

pub const PROTOCOL_VERSION: &str = "0.4.0";

/// An application error: a code outside JSON-RPC's reserved range, with a
/// name for the OpenRPC document.
pub struct AppError {
    pub name: &'static str,
    pub code: i64,
    pub message: &'static str,
}

pub const NOT_INITIALIZED: AppError = AppError {
    name: "NotInitialized",
    code: -32001,
    message: "call initialize first",
};
pub const ALREADY_INITIALIZED: AppError = AppError {
    name: "AlreadyInitialized",
    code: -32002,
    message: "already initialized",
};
pub const UNAUTHORIZED: AppError = AppError {
    name: "Unauthorized",
    code: -32003,
    message: "missing or wrong token; read it from this REAPER's instance file",
};
pub const ACTION_CONFLICT: AppError = AppError {
    name: "ActionConflict",
    code: -32010,
    message: "the action is owned by another connection",
};
pub const UNKNOWN_ACTION: AppError = AppError {
    name: "UnknownAction",
    code: -32011,
    message: "this connection has no action with that id",
};
pub const UNKNOWN_COMMAND: AppError = AppError {
    name: "UnknownCommand",
    code: -32012,
    message: "REAPER has no such command",
};
pub const REGISTRATION_FAILED: AppError = AppError {
    name: "RegistrationFailed",
    code: -32013,
    message: "REAPER refused the registration",
};

pub const NOT_FOUND: AppError = AppError {
    name: "NotFound",
    code: -32014,
    message: "nothing in the current project has that GUID",
};

pub const MEDIA_UNAVAILABLE: AppError = AppError {
    name: "MediaUnavailable",
    code: -32015,
    message: "REAPER can't open that media file",
};

pub const APP_ERRORS: &[&AppError] = &[
    &NOT_INITIALIZED,
    &ALREADY_INITIALIZED,
    &UNAUTHORIZED,
    &ACTION_CONFLICT,
    &UNKNOWN_ACTION,
    &UNKNOWN_COMMAND,
    &REGISTRATION_FAILED,
    &NOT_FOUND,
    &MEDIA_UNAVAILABLE,
];

#[derive(Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    /// Details for the client, sent as the error's `data`.
    pub data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(-32700, message)
    }
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(-32600, message)
    }
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }

    /// An application error with a more specific message.
    pub fn app(error: &AppError, message: impl Into<String>) -> Self {
        Self::new(error.code, message)
    }
    pub fn not_initialized() -> Self {
        Self::app(&NOT_INITIALIZED, NOT_INITIALIZED.message)
    }
    pub fn already_initialized() -> Self {
        Self::app(&ALREADY_INITIALIZED, ALREADY_INITIALIZED.message)
    }
}

/// Named params for one call. Absent params behave like an empty object.
pub struct Params(Map<String, Value>);

impl Params {
    pub fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

/// Handles one incoming text frame, which may be a single message or a batch.
/// `call` runs each request or notification. Returns the text to send back,
/// or `None` when nothing should be sent (only notifications).
pub fn handle_text(
    text: &str,
    mut call: impl FnMut(&str, Params) -> Result<Value, RpcError>,
) -> Option<String> {
    let message: Value = match serde_json::from_str(text) {
        Ok(message) => message,
        Err(error) => return Some(error_response(Value::Null, RpcError::parse_error(error.to_string())).to_string()),
    };
    match message {
        Value::Array(batch) if batch.is_empty() => {
            Some(error_response(Value::Null, RpcError::invalid_request("empty batch")).to_string())
        }
        Value::Array(batch) => {
            let replies: Vec<Value> = batch
                .into_iter()
                .filter_map(|message| handle_message(message, &mut call))
                .collect();
            (!replies.is_empty()).then(|| Value::Array(replies).to_string())
        }
        message => handle_message(message, &mut call).map(|reply| reply.to_string()),
    }
}

fn handle_message(
    message: Value,
    call: &mut impl FnMut(&str, Params) -> Result<Value, RpcError>,
) -> Option<Value> {
    let Value::Object(mut message) = message else {
        return Some(error_response(Value::Null, RpcError::invalid_request("expected an object")));
    };
    // A missing id makes this a notification; an explicit null is a request.
    let id = message.remove("id");
    if let Some(id) = &id {
        if !matches!(id, Value::String(_) | Value::Number(_) | Value::Null) {
            return Some(error_response(Value::Null, RpcError::invalid_request("id must be a string, number, or null")));
        }
    }
    let respond = |result: Result<Value, RpcError>| {
        let id = id.clone()?;
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => error_response(id, error),
        })
    };

    if message.get("jsonrpc") != Some(&json!("2.0")) {
        let error = RpcError::invalid_request("jsonrpc must be \"2.0\"");
        return Some(error_response(id.unwrap_or(Value::Null), error));
    }
    let Some(Value::String(method)) = message.remove("method") else {
        let error = RpcError::invalid_request("method must be a string");
        return Some(error_response(id.unwrap_or(Value::Null), error));
    };
    let params = match message.remove("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => return respond(Err(RpcError::invalid_params("params must be an object"))),
    };
    respond(call(&method, Params(params)))
}

fn error_response(id: Value, error: RpcError) -> Value {
    let mut body = json!({ "code": error.code, "message": error.message });
    if let Some(data) = error.data {
        body["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": body })
}

/// Serializes a server-to-client notification.
pub fn notification<T: serde::Serialize + ?Sized>(method: &str, params: &T) -> String {
    #[derive(serde::Serialize)]
    struct Notification<'a, T: ?Sized> {
        jsonrpc: &'static str,
        method: &'a str,
        params: &'a T,
    }
    serde_json::to_string(&Notification {
        jsonrpc: "2.0",
        method,
        params,
    })
    .expect("protocol types always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(method: &str, params: Params) -> Result<Value, RpcError> {
        match method {
            "echo" => Ok(Value::Object(params.0)),
            "fail" => Err(RpcError::app(&UNKNOWN_COMMAND, "nope")),
            other => Err(RpcError::method_not_found(other)),
        }
    }

    fn reply(text: &str) -> Value {
        serde_json::from_str(&handle_text(text, echo).expect("a reply")).unwrap()
    }

    #[test]
    fn request_gets_result() {
        let reply = reply(r#"{"jsonrpc":"2.0","id":1,"method":"echo","params":{"a":1}}"#);
        assert_eq!(reply, json!({ "jsonrpc": "2.0", "id": 1, "result": { "a": 1 } }));
    }

    #[test]
    fn missing_params_are_empty() {
        let reply = reply(r#"{"jsonrpc":"2.0","id":"x","method":"echo"}"#);
        assert_eq!(reply["result"], json!({}));
    }

    #[test]
    fn null_id_is_still_a_request() {
        let reply = reply(r#"{"jsonrpc":"2.0","id":null,"method":"echo"}"#);
        assert_eq!(reply["id"], Value::Null);
        assert_eq!(reply["result"], json!({}));
    }

    #[test]
    fn notification_gets_no_reply() {
        assert_eq!(handle_text(r#"{"jsonrpc":"2.0","method":"echo"}"#, echo), None);
        assert_eq!(handle_text(r#"{"jsonrpc":"2.0","method":"fail"}"#, echo), None);
    }

    #[test]
    fn application_error() {
        let reply = reply(r#"{"jsonrpc":"2.0","id":2,"method":"fail"}"#);
        assert_eq!(reply["error"]["code"], -32012);
        assert_eq!(reply["id"], 2);
    }

    #[test]
    fn standard_errors() {
        assert_eq!(reply("{oops")["error"]["code"], -32700);
        assert_eq!(reply(r#"{"id":1,"method":"echo"}"#)["error"]["code"], -32600);
        assert_eq!(reply(r#"{"jsonrpc":"2.0","id":1,"method":5}"#)["error"]["code"], -32600);
        assert_eq!(reply(r#"{"jsonrpc":"2.0","id":{},"method":"echo"}"#)["error"]["code"], -32600);
        assert_eq!(reply(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#)["error"]["code"], -32601);
        assert_eq!(reply(r#"{"jsonrpc":"2.0","id":1,"method":"echo","params":[1]}"#)["error"]["code"], -32602);
        assert_eq!(reply("[]")["error"]["code"], -32600);
    }

    #[test]
    fn batch() {
        let reply = reply(
            r#"[{"jsonrpc":"2.0","id":1,"method":"echo"},
                {"jsonrpc":"2.0","method":"echo"},
                {"jsonrpc":"2.0","id":2,"method":"nope"},
                3]"#,
        );
        let replies = reply.as_array().unwrap();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[1]["error"]["code"], -32601);
        assert_eq!(replies[2]["error"]["code"], -32600);
    }

    #[test]
    fn batch_of_notifications_gets_no_reply() {
        let text = r#"[{"jsonrpc":"2.0","method":"echo"},{"jsonrpc":"2.0","method":"echo"}]"#;
        assert_eq!(handle_text(text, echo), None);
    }
}
