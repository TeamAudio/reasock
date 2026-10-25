//! JSON-RPC 2.0 envelope handling, independent of REAPER.
//!
//! See https://www.jsonrpc.org/specification. Params are by-name only.

use serde_json::{json, Map, Value};

pub const PROTOCOL_VERSION: &str = "0.1.0";

/// Every method a client may call; kept in sync with openrpc.json by a test.
pub const METHODS: &[&str] = &[
    "initialize",
    "rpc.discover",
    "action.register",
    "action.unregister",
    "action.run",
    "events.subscribe",
    "events.unsubscribe",
    "debug.stats",
];

/// The OpenRPC description of the protocol, served by `rpc.discover`.
pub const OPENRPC: &str = include_str!("../openrpc.json");

#[derive(Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
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

    // Application errors, documented in openrpc.json.
    pub fn not_initialized() -> Self {
        Self::new(-32001, "call initialize first")
    }
    pub fn already_initialized() -> Self {
        Self::new(-32002, "already initialized")
    }
    pub fn action_conflict(message: impl Into<String>) -> Self {
        Self::new(-32010, message)
    }
    pub fn unknown_action(message: impl Into<String>) -> Self {
        Self::new(-32011, message)
    }
    pub fn unknown_command(message: impl Into<String>) -> Self {
        Self::new(-32012, message)
    }
    pub fn registration_failed(message: impl Into<String>) -> Self {
        Self::new(-32013, message)
    }
}

/// Named params for one call. Absent params behave like an empty object.
pub struct Params(Map<String, Value>);

impl Params {
    pub fn str(&self, key: &str) -> Result<&str, RpcError> {
        self.0
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params(format!("{key} must be a string")))
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
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
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message },
    })
}

pub fn notification(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(method: &str, params: Params) -> Result<Value, RpcError> {
        match method {
            "echo" => Ok(Value::Object(params.0)),
            "fail" => Err(RpcError::unknown_command("nope")),
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

    #[test]
    fn openrpc_document_matches_implementation() {
        let document: Value = serde_json::from_str(OPENRPC).unwrap();
        assert_eq!(document["info"]["version"], PROTOCOL_VERSION);
        let mut documented: Vec<&str> = document["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method["name"].as_str().unwrap())
            .collect();
        let mut implemented = METHODS.to_vec();
        documented.sort();
        implemented.sort();
        assert_eq!(documented, implemented);
    }
}
