//! Main-thread state and protocol handling.
//!
//! Everything here runs on REAPER's main thread, from either the timer or the
//! command hooks. Running an action re-enters the command hook synchronously,
//! so no lock may be held across `main_on_command_ex`.

use crate::log::log;
use crate::protocol::{self, Params, RpcError};
use crate::server::{ConnId, Inbound};
use reaper_medium::{
    ActionValueChange, CommandId, HookCommand, HookCommand2, ItemAttributeKey, ProjectContext,
    Reaper, ReaperSession, SectionContext, WindowContext,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, MutexGuard};

struct Connection {
    /// Set by `initialize`; used to namespace this connection's actions.
    client: Option<String>,
    outbox: Sender<String>,
}

struct Action {
    conn: ConnId,
    id: String,
    command_name: String,
    registration: Box<CustomAction>,
}

/// Actions are registered with `custom_action` rather than `command_id` +
/// `gaccel`: the latter loses the command name after an unregister/re-register
/// cycle, which REAPER then saves into reaper-kb.ini as a blank `_` binding.
///
/// Mirrors `custom_action_register_t` from reaper_plugin.h, which reaper-low
/// does not bind. Boxed so REAPER can hold on to a stable address.
#[repr(C)]
struct CustomActionRegister {
    unique_section_id: c_int,
    id_str: *const c_char,
    name: *const c_char,
    extra: *mut c_void,
}

struct CustomAction {
    register: CustomActionRegister,
    _id_str: CString,
    _name: CString,
}

// The raw pointers only ever point into the CStrings owned alongside them.
unsafe impl Send for CustomAction {}

impl CustomAction {
    fn remove(mut self: Box<Self>) {
        unsafe {
            Reaper::get().low().plugin_register(
                c"-custom_action".as_ptr(),
                &mut self.register as *mut CustomActionRegister as *mut c_void,
            );
        }
    }
}

#[derive(Default)]
struct State {
    connections: HashMap<ConnId, Connection>,
    actions: HashMap<CommandId, Action>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static INBOX: Mutex<Option<Receiver<Inbound>>> = Mutex::new(None);
/// Kept alive for the life of REAPER; dropping it unregisters the hooks.
static SESSION: Mutex<Option<ReaperSession>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(lock(&STATE).get_or_insert_with(State::default))
}

pub fn init(inbox: Receiver<Inbound>, session: ReaperSession) {
    *lock(&INBOX) = Some(inbox);
    *lock(&SESSION) = Some(session);
}

pub extern "C" fn on_timer() {
    let result = std::panic::catch_unwind(|| {
        let pending: Vec<Inbound> = match lock(&INBOX).as_ref() {
            Some(inbox) => inbox.try_iter().collect(),
            None => return,
        };
        for message in pending {
            handle_inbound(message);
        }
    });
    if result.is_err() {
        log!("panic in timer");
    }
}

fn handle_inbound(message: Inbound) {
    match message {
        Inbound::Connected { conn, outbox } => with_state(|state| {
            let connection = Connection { client: None, outbox };
            state.connections.insert(conn, connection);
        }),
        Inbound::Message { conn, text } => {
            let reply = protocol::handle_text(&text, |method, params| call(conn, method, params));
            if let Some(reply) = reply {
                send(conn, reply);
            }
        }
        Inbound::Disconnected { conn } => disconnect(conn),
    }
}

fn disconnect(conn: ConnId) {
    let removed: Vec<Action> = with_state(|state| {
        state.connections.remove(&conn);
        let commands: Vec<CommandId> = state
            .actions
            .iter()
            .filter(|(_, action)| action.conn == conn)
            .map(|(command, _)| *command)
            .collect();
        commands
            .into_iter()
            .filter_map(|command| state.actions.remove(&command))
            .collect()
    });
    for action in removed {
        action.registration.remove();
        log!("conn {conn}: unregistered _{}", action.command_name);
    }
}

fn send(conn: ConnId, text: String) {
    let outbox = with_state(|state| state.connections.get(&conn).map(|c| c.outbox.clone()));
    if let Some(outbox) = outbox {
        let _ = outbox.send(text);
    }
}

fn call(conn: ConnId, method: &str, params: Params) -> Result<Value, RpcError> {
    match method {
        "initialize" => return initialize(conn, &params),
        "rpc.discover" => return discover(),
        _ => {}
    }
    if !protocol::METHODS.contains(&method) {
        return Err(RpcError::method_not_found(method));
    }
    let client = with_state(|state| state.connections.get(&conn).and_then(|c| c.client.clone()))
        .ok_or_else(RpcError::not_initialized)?;
    match method {
        "action.register" => register_action(conn, &client, &params),
        "action.unregister" => unregister_action(conn, &params),
        "action.run" => run_action(&params),
        other => Err(RpcError::internal(format!("{other} is listed but not implemented"))),
    }
}

/// Command names may only contain A-Z, a-z and 0-9 (plus our underscores).
fn validate_name(value: &str, what: &str) -> Result<(), RpcError> {
    let valid = !value.is_empty()
        && value.len() <= 32
        && value.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(RpcError::invalid_params(format!(
            "{what} must be 1-32 ASCII letters or digits"
        )))
    }
}

fn initialize(conn: ConnId, params: &Params) -> Result<Value, RpcError> {
    params.str("protocolVersion")?;
    let client = params
        .get("client")
        .and_then(|client| client.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("client.name must be a string"))?;
    validate_name(client, "client.name")?;
    with_state(|state| {
        let connection = state
            .connections
            .get_mut(&conn)
            .ok_or_else(|| RpcError::internal("unknown connection"))?;
        if connection.client.is_some() {
            return Err(RpcError::already_initialized());
        }
        connection.client = Some(client.to_owned());
        Ok(())
    })?;
    log!("conn {conn}: initialized as {client}");
    Ok(json!({
        "protocolVersion": protocol::PROTOCOL_VERSION,
        "server": { "name": "reasock", "version": env!("CARGO_PKG_VERSION") },
        "reaper": { "version": reaper_version() },
        "connection": conn,
        "capabilities": { "actions": { "register": true, "run": true } },
    }))
}

fn discover() -> Result<Value, RpcError> {
    serde_json::from_str(protocol::OPENRPC).map_err(|error| RpcError::internal(error.to_string()))
}

fn register_action(conn: ConnId, client: &str, params: &Params) -> Result<Value, RpcError> {
    let id = params.str("id")?;
    let label = params.str("label")?;
    validate_name(id, "id")?;
    if label.is_empty() {
        return Err(RpcError::invalid_params("label must not be empty"));
    }
    let command_name = format!("REASOCK_{client}_{id}");

    let existing = with_state(|state| {
        state
            .actions
            .iter()
            .find(|(_, action)| action.command_name == command_name)
            .map(|(command, action)| (*command, action.conn))
    });
    match existing {
        Some((command, owner)) if owner == conn => {
            return Ok(json!({ "command": format!("_{command_name}"), "commandId": command.get() }));
        }
        Some(_) => {
            return Err(RpcError::action_conflict(format!(
                "_{command_name} is owned by another connection"
            )))
        }
        None => {}
    }

    let description = format!("ReaSock ({client}): {label}");
    let (command, registration) = register_custom_action(&command_name, &description)?;
    log!("conn {conn}: registered _{command_name} as {}", command.get());

    with_state(|state| {
        state.actions.insert(
            command,
            Action {
                conn,
                id: id.to_owned(),
                command_name: command_name.clone(),
                registration,
            },
        )
    });
    Ok(json!({ "command": format!("_{command_name}"), "commandId": command.get() }))
}

fn register_custom_action(
    command_name: &str,
    description: &str,
) -> Result<(CommandId, Box<CustomAction>), RpcError> {
    let id_str = CString::new(command_name).map_err(|e| RpcError::invalid_params(e.to_string()))?;
    let name = CString::new(description).map_err(|e| RpcError::invalid_params(e.to_string()))?;
    let mut action = Box::new(CustomAction {
        register: CustomActionRegister {
            unique_section_id: 0,
            id_str: id_str.as_ptr(),
            name: name.as_ptr(),
            extra: std::ptr::null_mut(),
        },
        _id_str: id_str,
        _name: name,
    });
    let raw_id = unsafe {
        Reaper::get().low().plugin_register(
            c"custom_action".as_ptr(),
            &mut action.register as *mut CustomActionRegister as *mut c_void,
        )
    };
    if raw_id <= 0 {
        return Err(RpcError::registration_failed(format!(
            "REAPER refused _{command_name}"
        )));
    }
    Ok((CommandId::new(raw_id as u32), action))
}

fn unregister_action(conn: ConnId, params: &Params) -> Result<Value, RpcError> {
    let id = params.str("id")?;
    let removed = with_state(|state| {
        let command = state
            .actions
            .iter()
            .find(|(_, action)| action.conn == conn && action.id == id)
            .map(|(command, _)| *command)?;
        state.actions.remove(&command)
    });
    let action =
        removed.ok_or_else(|| RpcError::unknown_action(format!("no action with id {id}")))?;
    action.registration.remove();
    log!("conn {conn}: unregistered _{}", action.command_name);
    Ok(json!({}))
}

fn run_action(params: &Params) -> Result<Value, RpcError> {
    let reaper = Reaper::get();
    let command = match params.get("command") {
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .map(CommandId::new),
        Some(Value::String(name)) => reaper.named_command_lookup(name.as_str()),
        _ => {
            return Err(RpcError::invalid_params(
                "command must be a command ID or a named command",
            ))
        }
    }
    .ok_or_else(|| RpcError::unknown_command(format!("unknown command: {}", params.get("command").unwrap())))?;
    // REAPER keeps resolving names of unregistered actions (that's how their
    // key bindings survive), so check the action list instead.
    let available = unsafe {
        reaper.kbd_get_text_from_cmd(command, SectionContext::MainSection, |text| {
            !text.to_str().is_empty()
        })
    };
    if available != Some(true) {
        return Err(RpcError::unknown_command(format!(
            "command {} is not currently available",
            command.get()
        )));
    }
    // Re-enters `Hook::call` for our own actions; no locks are held here.
    reaper.main_on_command_ex(command, 0, ProjectContext::CurrentProject);
    Ok(json!({ "commandId": command.get() }))
}

pub struct Hook;

/// Sees actions run from the main section, including `Main_OnCommand`.
impl HookCommand for Hook {
    fn call(command: CommandId, _flag: i32) -> bool {
        dispatch(command)
    }
}

/// Sees actions triggered by key/MIDI/OSC; custom actions only arrive here.
impl HookCommand2 for Hook {
    fn call(
        _section: SectionContext,
        command: CommandId,
        _value_change: ActionValueChange,
        _window: WindowContext,
    ) -> bool {
        dispatch(command)
    }
}

fn dispatch(command: CommandId) -> bool {
    let target = with_state(|state| {
        let action = state.actions.get(&command)?;
        let connection = state.connections.get(&action.conn)?;
        Some((
            connection.outbox.clone(),
            action.id.clone(),
            action.command_name.clone(),
        ))
    });
    let Some((outbox, id, command_name)) = target else {
        return false;
    };
    let message = protocol::notification(
        "action.triggered",
        json!({
            "id": id,
            "command": format!("_{command_name}"),
            "context": { "selectedItems": selected_items() },
        }),
    );
    let _ = outbox.send(message);
    true
}

fn selected_items() -> Vec<Value> {
    let reaper = Reaper::get();
    let project = ProjectContext::CurrentProject;
    let count = reaper.count_selected_media_items(project);
    (0..count)
        .filter_map(|index| reaper.get_selected_media_item(project, index))
        .map(|item| unsafe {
            let track = reaper
                .get_media_item_track(item)
                .and_then(|track| {
                    reaper.get_set_media_track_info_get_name(track, |name| {
                        name.to_str().to_owned()
                    })
                });
            let path = reaper
                .get_active_take(item)
                .and_then(|take| reaper.get_media_item_take_source(take))
                .map(|source| {
                    let mut buffer = [0 as c_char; 4096];
                    reaper.low().GetMediaSourceFileName(
                        source.as_ptr(),
                        buffer.as_mut_ptr(),
                        buffer.len() as i32,
                    );
                    CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned()
                })
                .filter(|path| !path.is_empty());
            json!({
                "path": path,
                "track": track,
                "position": reaper.get_media_item_info_value(item, ItemAttributeKey::Position),
                "length": reaper.get_media_item_info_value(item, ItemAttributeKey::Length),
            })
        })
        .collect()
}

pub fn reaper_version() -> String {
    let ptr = Reaper::get().low().GetAppVersion();
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}
