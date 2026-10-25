mod bridge;
mod discovery;
mod log;
mod protocol;
mod server;

use crate::log::log;
use reaper_low::PluginContext;
use reaper_macros::reaper_extension_plugin;
use reaper_medium::{Reaper, ReaperSession};
use std::error::Error;
use std::sync::mpsc;

#[reaper_extension_plugin]
fn plugin_main(context: PluginContext) -> Result<(), Box<dyn Error>> {
    let mut session = ReaperSession::load(context);
    let _ = Reaper::make_available_globally(session.reaper().clone());

    let (inbox_sender, inbox) = mpsc::channel();
    let port = server::start(inbox_sender)?;

    session.plugin_register_add_hook_command::<bridge::Hook>()?;
    session.plugin_register_add_hook_command_2::<bridge::Hook>()?;
    session.plugin_register_add_timer(bridge::on_timer)?;
    session.plugin_register_add_at_exit(on_exit)?;

    let reaper_version = bridge::reaper_version();
    let instance = discovery::write(port, &reaper_version)?;
    bridge::init(inbox, session);

    log!(
        "ReaSock {} loaded in REAPER {reaper_version}; listening on 127.0.0.1:{port}; announced at {}",
        env!("CARGO_PKG_VERSION"),
        instance.display()
    );
    Ok(())
}

extern "C" fn on_exit() {
    discovery::remove();
    log!("REAPER exiting");
}
