//! Main-thread state and protocol handling.
//!
//! Everything here runs on REAPER's main thread, from either the timer or the
//! command hooks. Running an action re-enters the command hook synchronously,
//! so no lock may be held across `main_on_command_ex`.

use crate::api;
use crate::collection::{self, Options};
use crate::events::Tracker;
use crate::log::log;
use crate::model;
use crate::protocol::{self, RpcError};
use crate::server::{ConnId, Inbound};
use crate::types::{
    ActionCapabilities, ActionContext, ActionTriggered, Capabilities, CommandRef, DebugStats,
    DebugStatsParams, Empty, EventCapabilities, EventName, InitializeParams, InitializeResult,
    RanAction, ReaperInfo, RegisterActionParams, RegisteredAction, RunActionParams, ServerInfo,
    Subscription, SubscriptionParams, Subscriptions, UnregisterActionParams,
};
use reaper_medium::{
    ActionValueChange, CommandId, HookCommand, HookCommand2, ProjectContext, Reaper,
    ReaperSession, SectionContext, WindowContext,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, MutexGuard};

struct Connection {
    /// Set by `initialize`; used to namespace this connection's actions.
    client: Option<String>,
    outbox: Sender<String>,
    subscriptions: BTreeMap<EventName, Options>,
    /// Subscribed since the last tick; owed the current snapshot.
    pending_initial: BTreeSet<EventName>,
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
static TRACKER: Mutex<Option<Tracker>> = Mutex::new(None);
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
        poll_events();
    });
    if result.is_err() {
        log!("panic in timer");
    }
}

fn handle_inbound(message: Inbound) {
    match message {
        Inbound::Connected { conn, outbox } => with_state(|state| {
            let connection = Connection {
                client: None,
                outbox,
                subscriptions: BTreeMap::new(),
                pending_initial: BTreeSet::new(),
            };
            state.connections.insert(conn, connection);
        }),
        Inbound::Message { conn, text } => {
            let reply =
                protocol::handle_text(&text, |method, params| api::call(conn, method, params));
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

/// This connection's client name, once it has called `initialize`.
pub fn client(conn: ConnId) -> Option<String> {
    with_state(|state| state.connections.get(&conn).and_then(|c| c.client.clone()))
}

/// Names must be usable inside REAPER command names; see `types::NAME_PATTERN`.
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

pub fn initialize(conn: ConnId, params: InitializeParams) -> Result<InitializeResult, RpcError> {
    let client = params.client.name;
    validate_name(&client, "client.name")?;
    let version = params
        .client
        .version
        .map(|version| format!(" {version}"))
        .unwrap_or_default();
    with_state(|state| {
        let connection = state
            .connections
            .get_mut(&conn)
            .ok_or_else(|| RpcError::internal("unknown connection"))?;
        if connection.client.is_some() {
            return Err(RpcError::already_initialized());
        }
        connection.client = Some(client.clone());
        Ok(())
    })?;
    log!(
        "conn {conn}: initialized as {client}{version} (protocol {})",
        params.protocol_version
    );
    Ok(InitializeResult {
        protocol_version: protocol::PROTOCOL_VERSION.into(),
        server: ServerInfo {
            name: "reasock".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        reaper: ReaperInfo {
            version: model::reaper_version(),
        },
        connection: conn,
        capabilities: Capabilities {
            actions: ActionCapabilities {
                register: true,
                run: true,
            },
            events: EventCapabilities {
                available: EventName::ALL.to_vec(),
            },
        },
    })
}

pub fn register_action(
    conn: ConnId,
    client: &str,
    params: RegisterActionParams,
) -> Result<RegisteredAction, RpcError> {
    let RegisterActionParams { id, label } = params;
    validate_name(&id, "id")?;
    if label.is_empty() {
        return Err(RpcError::invalid_params("label must not be empty"));
    }
    let command_name = format!("REASOCK_{client}_{id}");
    let registered = |command: CommandId| RegisteredAction {
        command: format!("_{command_name}"),
        command_id: command.get(),
    };

    let existing = with_state(|state| {
        state
            .actions
            .iter()
            .find(|(_, action)| action.command_name == command_name)
            .map(|(command, action)| (*command, action.conn))
    });
    match existing {
        Some((command, owner)) if owner == conn => return Ok(registered(command)),
        Some(_) => {
            return Err(RpcError::app(
                &protocol::ACTION_CONFLICT,
                format!("_{command_name} is owned by another connection"),
            ))
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
                id,
                command_name: command_name.clone(),
                registration,
            },
        )
    });
    Ok(registered(command))
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
        return Err(RpcError::app(
            &protocol::REGISTRATION_FAILED,
            format!("REAPER refused _{command_name}"),
        ));
    }
    Ok((CommandId::new(raw_id as u32), action))
}

pub fn unregister_action(conn: ConnId, params: UnregisterActionParams) -> Result<Empty, RpcError> {
    let id = params.id;
    validate_name(&id, "id")?;
    let removed = with_state(|state| {
        let command = state
            .actions
            .iter()
            .find(|(_, action)| action.conn == conn && action.id == id)
            .map(|(command, _)| *command)?;
        state.actions.remove(&command)
    });
    let action = removed.ok_or_else(|| {
        RpcError::app(&protocol::UNKNOWN_ACTION, format!("no action with id {id}"))
    })?;
    action.registration.remove();
    log!("conn {conn}: unregistered _{}", action.command_name);
    Ok(Empty {})
}

pub fn run_action(params: RunActionParams) -> Result<RanAction, RpcError> {
    let reaper = Reaper::get();
    let unknown = |what: String| RpcError::app(&protocol::UNKNOWN_COMMAND, what);
    let command = match &params.command {
        CommandRef::Id(0) => return Err(RpcError::invalid_params("command IDs start at 1")),
        CommandRef::Name(name) if name.is_empty() => {
            return Err(RpcError::invalid_params("command must not be empty"))
        }
        CommandRef::Id(id) => Some(CommandId::new(*id)),
        CommandRef::Name(name) => reaper.named_command_lookup(name.as_str()),
    }
    .ok_or_else(|| unknown(format!("unknown command: {:?}", params.command)))?;
    // REAPER keeps resolving names of unregistered actions (that's how their
    // key bindings survive), so check the action list instead.
    let available = unsafe {
        reaper.kbd_get_text_from_cmd(command, SectionContext::MainSection, |text| {
            !text.to_str().is_empty()
        })
    };
    if available != Some(true) {
        return Err(unknown(format!(
            "command {} is not currently available",
            command.get()
        )));
    }
    // Re-enters `Hook::call` for our own actions; no locks are held here.
    reaper.main_on_command_ex(command, 0, ProjectContext::CurrentProject);
    Ok(RanAction {
        command_id: command.get(),
    })
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
    let params = ActionTriggered {
        id,
        command: format!("_{command_name}"),
        context: action_context(),
    };
    let _ = outbox.send(protocol::notification("action.triggered", &params));
    true
}

/// Selected items when an action runs, capped like collection notifications.
fn action_context() -> ActionContext {
    let (count, items) = model::selected_items_page(0, collection::DEFAULT_LIMIT);
    ActionContext {
        truncated: count > items.len(),
        selected_items: items,
        selected_item_count: count,
    }
}

fn require_events(params: &SubscriptionParams) -> Result<(), RpcError> {
    if params.events.is_empty() {
        return Err(RpcError::invalid_params("events must not be empty"));
    }
    Ok(())
}

pub fn subscribe(conn: ConnId, params: SubscriptionParams) -> Result<Subscriptions, RpcError> {
    require_events(&params)?;
    let mut requested = Vec::new();
    for subscription in &params.events {
        let event = subscription.event();
        let options = match subscription {
            Subscription::WithOptions {
                options: Some(options),
                ..
            } => {
                if !event.is_collection() {
                    return Err(RpcError::invalid_params(format!(
                        "{} takes no options",
                        event.as_str()
                    )));
                }
                Options::try_from(options).map_err(|message| {
                    RpcError::invalid_params(format!("{}: {message}", event.as_str()))
                })?
            }
            _ => Options::default(),
        };
        requested.push((event, options));
    }
    with_state(|state| {
        let connection = state
            .connections
            .get_mut(&conn)
            .ok_or_else(|| RpcError::internal("unknown connection"))?;
        for (event, options) in requested {
            // New subscriptions, and changed options, start with a full picture.
            if connection.subscriptions.insert(event, options) != Some(options) {
                connection.pending_initial.insert(event);
            }
        }
        Ok(Subscriptions {
            events: connection.subscriptions.keys().copied().collect(),
        })
    })
}

pub fn unsubscribe(conn: ConnId, params: SubscriptionParams) -> Result<Subscriptions, RpcError> {
    require_events(&params)?;
    with_state(|state| {
        let connection = state
            .connections
            .get_mut(&conn)
            .ok_or_else(|| RpcError::internal("unknown connection"))?;
        for subscription in &params.events {
            connection.subscriptions.remove(&subscription.event());
            connection.pending_initial.remove(&subscription.event());
        }
        Ok(Subscriptions {
            events: connection.subscriptions.keys().copied().collect(),
        })
    })
}

pub fn debug_stats(params: DebugStatsParams) -> DebugStats {
    let mut tracker = lock(&TRACKER);
    let tracker = tracker.get_or_insert_with(Tracker::default);
    let stats = tracker.stats.report();
    if params.reset.unwrap_or(false) {
        tracker.stats = Default::default();
    }
    stats
}

/// Sends changed state to subscribers, and current state to new subscribers.
fn poll_events() {
    let started = std::time::Instant::now();
    type Wanted = Vec<(EventName, bool, Options)>;
    let subscribers: Vec<(Sender<String>, Wanted)> = with_state(|state| {
        state
            .connections
            .values_mut()
            .filter(|connection| !connection.subscriptions.is_empty())
            .map(|connection| {
                let wanted = connection
                    .subscriptions
                    .iter()
                    .map(|(&event, &options)| {
                        (event, connection.pending_initial.contains(&event), options)
                    })
                    .collect();
                connection.pending_initial.clear();
                (connection.outbox.clone(), wanted)
            })
            .collect()
    });
    let wanted: BTreeSet<EventName> = subscribers
        .iter()
        .flat_map(|(_, events)| events.iter().map(|(event, _, _)| *event))
        .collect();
    let wanted: Vec<EventName> = wanted.into_iter().collect();
    let mut tracker = lock(&TRACKER);
    let tracker = tracker.get_or_insert_with(Tracker::default);
    // Still called with nothing wanted, so stale snapshots are forgotten.
    let changed = tracker.poll(&wanted);
    if wanted.is_empty() {
        return;
    }
    // Serialize once per distinct rendering, however many connections share it.
    let mut messages: HashMap<(EventName, bool, Options), String> = HashMap::new();
    for (outbox, events) in subscribers {
        for (event, initial, options) in events {
            if !(initial || changed.contains(&event)) {
                continue;
            }
            let key = (event, initial, options);
            if !messages.contains_key(&key) {
                let Some(message) = tracker.message(event, initial, options) else {
                    continue;
                };
                messages.insert(key, message);
            }
            let message = &messages[&key];
            tracker.stats.record_send(event, message.len());
            let _ = outbox.send(message.clone());
        }
    }
    tracker.stats.record_tick(started.elapsed());
}
