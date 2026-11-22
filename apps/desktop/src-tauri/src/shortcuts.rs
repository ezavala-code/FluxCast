use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::error::Error;
use tauri::{AppHandle, Emitter, Manager};

const ACTION: &str = "toggle_launcher";

pub fn status(app: &AppHandle, text: &str) {
    *app.state::<super::Launcher>().shortcut.lock().unwrap() = text.to_owned();
    let _ = app.emit("shortcut-status", text);
}

pub async fn run(app: AppHandle) -> Result<(), Box<dyn Error + Send + Sync>> {
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session().await?;
    // ashpd representa Session como un object path de D-Bus al serializarlo.
    let path = serde_json::to_value(&session)?;
    let mut activated = portal.receive_activated().await?;
    let closed = session.receive_closed().await?;
    let previous = portal.list_shortcuts(&session).await?.response()?;
    let bindings = if previous.shortcuts().iter().any(|s| s.id() == ACTION) {
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
    let binding = bindings
        .iter()
        .find(|s| s.id() == ACTION && !s.trigger_description().is_empty())
        .ok_or("KDE no asignó una combinación")?;
    status(&app, binding.trigger_description());
    let mut events = (&mut activated).take_until(closed.into_future());
    while let Some(event) = events.next().await {
        if event.shortcut_id() == ACTION && Some(event.session_handle().as_str()) == path.as_str() {
            let token = event
                .options()
                .get("activation_token")
                .and_then(|v| <&str>::try_from(v).ok())
                .map(str::to_owned);
            super::schedule_toggle(&app, token, None)?;
        }
    }
    Err("KDE cerró la sesión de atajos".into())
}
