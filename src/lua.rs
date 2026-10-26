//! The ReaScript API: lets scripts inside REAPER use the protocol through a
//! local connection, exactly as a WebSocket client would.
//!
//! ```lua
//! local conn = reaper.ReaSock_Open("myscript")
//! local reply = reaper.ReaSock_Call(conn, "tracks.list", '{"limit": 10}')
//! reaper.ReaSock_Send(conn, '{"jsonrpc":"2.0","method":"events.subscribe","params":{...}}')
//! local message = reaper.ReaSock_Poll(conn)  -- "" when nothing is waiting
//! reaper.ReaSock_Close(conn)
//! ```
//!
//! Each function has a C entry point (registered as `API_*`) and a vararg
//! wrapper for ReaScript (`APIvararg_*`), following reaspeech-lib.

use crate::bridge;
use crate::log::log;
use reaper_medium::ReaperSession;
use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn register(session: &mut ReaperSession) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        session.plugin_register_add_api_and_def(
            "ReaSock_Open",
            open as *mut c_void,
            open_vararg,
            "int",
            "const char*",
            "client",
            "Opens a local ReaSock connection and initializes it as `client` (1-32 letters or \
             digits). Returns the connection, or 0 if that failed. A connection that goes 10 \
             seconds without a Call or Poll is closed.",
        )?;
        session.plugin_register_add_api_and_def(
            "ReaSock_Call",
            call as *mut c_void,
            call_vararg,
            "const char*",
            "int,const char*,const char*",
            "connection,method,paramsOptional",
            "Runs one ReaSock method with JSON params and returns the JSON-RPC response, with \
             either `result` or `error`.",
        )?;
        session.plugin_register_add_api_and_def(
            "ReaSock_Send",
            send as *mut c_void,
            send_vararg,
            "bool",
            "int,const char*",
            "connection,message",
            "Sends any JSON-RPC message or batch. Replies arrive through ReaSock_Poll. Returns \
             false if the connection is closed.",
        )?;
        session.plugin_register_add_api_and_def(
            "ReaSock_Poll",
            poll as *mut c_void,
            poll_vararg,
            "const char*",
            "int",
            "connection",
            "Returns the next waiting message (a reply, event, or action.triggered), or an \
             empty string.",
        )?;
        session.plugin_register_add_api_and_def(
            "ReaSock_Close",
            close as *mut c_void,
            close_vararg,
            "void",
            "int",
            "connection",
            "Closes a local connection, removing its actions and subscriptions.",
        )?;
    }
    Ok(())
}

/// Runs `f`, turning a panic into `fallback` so it never unwinds into REAPER.
fn guarded<T>(name: &str, fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log!("panic in {name}");
        fallback
    })
}

thread_local! {
    /// Backs the string a function returns until that function's next call.
    static RETURNED: RefCell<CString> = RefCell::new(CString::default());
}

fn return_string(value: String) -> *const c_char {
    let value = CString::new(value.replace('\0', "\u{fffd}")).expect("NULs were replaced");
    RETURNED.with(|slot| {
        *slot.borrow_mut() = value;
        slot.borrow().as_ptr()
    })
}

unsafe fn text<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    CStr::from_ptr(ptr).to_str().unwrap_or("")
}

fn connection(value: c_int) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

extern "C" fn open(client: *const c_char) -> c_int {
    guarded("ReaSock_Open", 0, || {
        match bridge::open_local(unsafe { text(client) }) {
            Ok(conn) => c_int::try_from(conn).unwrap_or(0),
            Err(message) => {
                log!("ReaSock_Open failed: {message}");
                0
            }
        }
    })
}

extern "C" fn call(conn: c_int, method: *const c_char, params: *const c_char) -> *const c_char {
    let reply = guarded("ReaSock_Call", String::new(), || {
        bridge::call_local(connection(conn), unsafe { text(method) }, unsafe { text(params) })
    });
    return_string(reply)
}

extern "C" fn send(conn: c_int, message: *const c_char) -> bool {
    guarded("ReaSock_Send", false, || {
        bridge::send_local(connection(conn), unsafe { text(message) })
    })
}

extern "C" fn poll(conn: c_int) -> *const c_char {
    let message = guarded("ReaSock_Poll", String::new(), || bridge::poll_local(connection(conn)));
    return_string(message)
}

extern "C" fn close(conn: c_int) {
    guarded("ReaSock_Close", (), || bridge::close_local(connection(conn)));
}

// ReaScript passes arguments as an array of pointers: strings as pointers,
// ints and bools as pointer-sized integers.

unsafe fn arg(args: *mut *mut c_void, count: c_int, index: usize) -> *mut c_void {
    if args.is_null() || index >= count.max(0) as usize {
        return std::ptr::null_mut();
    }
    *args.add(index)
}

unsafe extern "C" fn open_vararg(args: *mut *mut c_void, count: c_int) -> *mut c_void {
    open(arg(args, count, 0).cast()) as isize as *mut c_void
}

unsafe extern "C" fn call_vararg(args: *mut *mut c_void, count: c_int) -> *mut c_void {
    call(
        arg(args, count, 0) as isize as c_int,
        arg(args, count, 1).cast(),
        arg(args, count, 2).cast(),
    ) as *mut c_void
}

unsafe extern "C" fn send_vararg(args: *mut *mut c_void, count: c_int) -> *mut c_void {
    send(arg(args, count, 0) as isize as c_int, arg(args, count, 1).cast()) as isize as *mut c_void
}

unsafe extern "C" fn poll_vararg(args: *mut *mut c_void, count: c_int) -> *mut c_void {
    poll(arg(args, count, 0) as isize as c_int) as *mut c_void
}

unsafe extern "C" fn close_vararg(args: *mut *mut c_void, count: c_int) -> *mut c_void {
    close(arg(args, count, 0) as isize as c_int);
    std::ptr::null_mut()
}
