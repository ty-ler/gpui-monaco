//! Commands by id (Monaco `CommandsRegistry`), run by code actions and
//! other providers' [`Command`]s.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Global, SharedString};

use crate::providers::Command;

type Handler = Rc<dyn Fn(&[serde_json::Value], &mut App)>;

#[derive(Default)]
struct Commands {
    handlers: HashMap<SharedString, Handler>,
}

impl Global for Commands {}

/// Registers `handler` for command `id` (Monaco
/// `CommandsRegistry.registerCommand`), replacing any earlier one.
pub fn register_command(
    cx: &mut App,
    id: impl Into<SharedString>,
    handler: impl Fn(&[serde_json::Value], &mut App) + 'static,
) {
    cx.default_global::<Commands>()
        .handlers
        .insert(id.into(), Rc::new(handler));
}

/// Removes command `id`.
pub fn unregister_command(cx: &mut App, id: &str) {
    cx.default_global::<Commands>().handlers.remove(id);
}

/// Runs `command`. Fails if nothing is registered for its id.
pub fn execute_command(cx: &mut App, command: &Command) -> anyhow::Result<()> {
    let handler = cx
        .try_global::<Commands>()
        .and_then(|commands| commands.handlers.get(&command.id).cloned())
        .ok_or_else(|| anyhow::anyhow!("command '{}' not found", command.id))?;
    handler(&command.arguments, cx);
    Ok(())
}
