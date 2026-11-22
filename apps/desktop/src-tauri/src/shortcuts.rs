use ashpd::desktop::global_shortcuts::{
    Activated, GlobalShortcuts, NewShortcut, Shortcut, ShortcutsChanged,
};
use futures_util::{stream, StreamExt};
use std::{error::Error, pin::pin};
use tauri::{AppHandle, Emitter, Manager};

const ACTION: &str = "toggle_launcher";

enum PortalEvent {
    Activated(Activated),
    Changed(ShortcutsChanged),
}

pub fn status(app: &AppHandle, text: &str) {
    *app.state::<super::Launcher>().shortcut.lock().unwrap() = text.to_owned();
    let _ = app.emit("shortcut-status", text);
}

fn update_shortcut_status(app: &AppHandle, shortcuts: &[Shortcut]) {
    let label = shortcuts
        .iter()
        .find(|s| s.id() == ACTION && !s.trigger_description().is_empty())
        .map(Shortcut::trigger_description)
        .unwrap_or("Sin asignar · fluxcast toggle");
    status(app, label);
}

pub async fn run(app: AppHandle) -> Result<(), Box<dyn Error + Send + Sync>> {
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session().await?;
    // ashpd representa Session como un object path de D-Bus al serializarlo.
    let path = serde_json::to_value(&session)?;
    let activated = pin!(portal
        .receive_activated()
        .await?
        .map(PortalEvent::Activated));
    let changed = pin!(portal
        .receive_shortcuts_changed()
        .await?
        .map(PortalEvent::Changed));
    let closed = session.receive_closed().await?;
    let previous = portal.list_shortcuts(&session).await?.response()?;
    let bindings = if previous
        .shortcuts()
        .iter()
        .any(|s| s.id() == ACTION && !s.trigger_description().is_empty())
    {
        previous.shortcuts().to_vec()
    } else {
        let action =
            NewShortcut::new(ACTION, "Mostrar u ocultar FluxCast").preferred_trigger("ALT+space");
        portal
            .bind_shortcuts(&session, &[action], None)
            .await?
            .response()?
            .shortcuts()
            .to_vec()
    };
    update_shortcut_status(&app, &bindings);
    let mut events = stream::select(activated, changed).take_until(closed.into_future());
    while let Some(event) = events.next().await {
        match event {
            PortalEvent::Activated(activated) => {
                if activated.shortcut_id() == ACTION
                    && Some(activated.session_handle().as_str()) == path.as_str()
                {
                    let token = activated
                        .options()
                        .get("activation_token")
                        .and_then(|v| <&str>::try_from(v).ok())
                        .map(str::to_owned);
                    super::schedule_toggle(&app, token, None)?;
                }
            }
            PortalEvent::Changed(changed) => {
                if Some(changed.session_handle().as_str()) == path.as_str() {
                    update_shortcut_status(&app, changed.shortcuts());
                }
            }
        }
    }
    Err("KDE cerró la sesión de atajos".into())
}
