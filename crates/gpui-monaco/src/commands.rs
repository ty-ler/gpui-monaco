//! Commands by id (Monaco `CommandsRegistry`), run by code actions and
//! other providers' [`Command`]s.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Global, SharedString};

use crate::providers::Command;
use crate::registry::Disposable;

type Handler = Rc<dyn Fn(&[serde_json::Value], &mut App)>;

/// Each id's handlers, newest last; the newest runs (Monaco keeps a
/// `LinkedList` per id).
#[derive(Default)]
struct Commands {
    next_id: u64,
    handlers: HashMap<SharedString, Vec<(u64, Handler)>>,
}

impl Global for Commands {}

/// Registers `handler` for command `id` (Monaco
/// `CommandsRegistry.registerCommand`) until the returned [`Disposable`] is
/// disposed; dropping it keeps the command. As in Monaco, it shadows
/// earlier handlers for `id`, which run again once it's disposed.
pub fn register_command(
    cx: &mut App,
    id: impl Into<SharedString>,
    handler: impl Fn(&[serde_json::Value], &mut App) + 'static,
) -> Disposable {
    add_command(cx, id, handler)
}

/// Adds `handler` for command `id` (Monaco `editor.addCommand`, which is
/// [`register_command`]) until the returned [`Disposable`] is disposed. It
/// shadows earlier handlers for `id`, which run again once it's disposed.
pub fn add_command(
    cx: &mut App,
    id: impl Into<SharedString>,
    handler: impl Fn(&[serde_json::Value], &mut App) + 'static,
) -> Disposable {
    let id = id.into();
    let commands = cx.default_global::<Commands>();
    commands.next_id += 1;
    let entry_id = commands.next_id;
    commands
        .handlers
        .entry(id.clone())
        .or_default()
        .push((entry_id, Rc::new(handler)));
    Disposable::new(move |cx| {
        let commands = cx.default_global::<Commands>();
        if let Some(handlers) = commands.handlers.get_mut(&id) {
            handlers.retain(|(i, _)| *i != entry_id);
            if handlers.is_empty() {
                commands.handlers.remove(&id);
            }
        }
    })
}

/// Removes command `id`.
pub fn unregister_command(cx: &mut App, id: &str) {
    cx.default_global::<Commands>().handlers.remove(id);
}

/// Whether a handler is registered for command `id`.
pub(crate) fn has_command(cx: &App, id: &str) -> bool {
    cx.try_global::<Commands>()
        .is_some_and(|commands| commands.handlers.contains_key(id))
}

/// Runs `command`. Fails if nothing is registered for its id.
pub fn execute_command(cx: &mut App, command: &Command) -> anyhow::Result<()> {
    let handler = cx
        .try_global::<Commands>()
        .and_then(|commands| commands.handlers.get(&command.id)?.last())
        .map(|(_, handler)| handler.clone())
        .ok_or_else(|| anyhow::anyhow!("command '{}' not found", command.id))?;
    handler(&command.arguments, cx);
    Ok(())
}
