mod client;
mod clipboard;
mod ipc_socket;
mod launcher;
mod shortcuts;

use gtk::prelude::GtkWindowExt;
use launcher::Launcher;
use std::{env, io, path::PathBuf, sync::mpsc, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

fn schedule_toggle(
    app: &AppHandle,
    token: Option<String>,
    ack: Option<mpsc::Sender<bool>>,
) -> tauri::Result<()> {
    let window = app
        .get_webview_window("main")
        .expect("ventana principal configurada");
    app.run_on_main_thread(move || {
        let toggle = || -> tauri::Result<()> {
            if window.is_visible()? {
                return window.hide();
            }
            let native = window.gtk_window()?;
            if let Some(token) = token {
                native.set_startup_id(&token);
            }
            window.show()?;
            native.present();
            window.set_focus()?;
            window.emit("launcher-shown", ())
        };
        let result = toggle();
        if let Err(err) = &result {
            eprintln!("FluxCast: {err}");
        }
        if let Some(ack) = ack {
            let _ = ack.send(result.is_ok());
        }
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").ok_or("Falta XDG_RUNTIME_DIR")?);
    let server = match ipc_socket::Server::bind(&runtime) {
        Ok(server) => server,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => return Ok(client::toggle(&runtime)?),
        Err(e) => return Err(e.into()),
    };

    tauri::Builder::default()
        .manage(Launcher::default())
        .invoke_handler(tauri::generate_handler![
            launcher::search,
            launcher::open_result,
            launcher::refresh_catalog,
            launcher::hide_launcher,
            launcher::copy_value,
            launcher::shortcut_status,
            launcher::list_clipboard,
            launcher::copy_clipboard_item,
            launcher::toggle_pin_clipboard,
            launcher::delete_clipboard_item,
            launcher::clear_clipboard_history,
            launcher::clipboard_pause_status,
            launcher::set_clipboard_pause,
            launcher::list_snippets,
            launcher::save_snippet,
            launcher::use_snippet,
            launcher::delete_snippet
        ])
        .setup(move |app| {
            launcher::warm_catalog(app.handle().clone());
            {
                let state = app.state::<Launcher>();
                clipboard::spawn_monitor(
                    app.handle().clone(),
                    Arc::clone(&state.store),
                    Arc::clone(&state.pause),
                );
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                server.serve(|| {
                    let (tx, rx) = mpsc::channel();
                    schedule_toggle(&handle, None, Some(tx)).is_ok()
                        && rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
                })
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = shortcuts::run(handle.clone()).await {
                    eprintln!("FluxCast · GlobalShortcuts: {error}");
                    shortcuts::status(&handle, "fluxcast toggle");
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(move |_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let _ = std::fs::remove_file(runtime.join("fluxcast.sock"));
            }
        });
    Ok(())
}
