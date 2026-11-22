use crate::{
    clipboard::{PauseController, PauseStatus},
    sync_worker::{PairingQrView, SyncCycleReport, SyncStatus, SyncWorker},
};
use fluxcast_core::{
    catalog::{Catalog, Kind},
    classifier::plain_text,
    desktop::system_catalog,
    store::{ClipboardItem, Snippet, SnippetInput, Store},
    utilities::{evaluate, Output},
};
use fluxcast_protocol::DeviceInfo;
use gio::prelude::AppInfoExt;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, UdpSocket},
    path::Path,
    process::{Child, Command},
    sync::{Arc, Mutex, RwLock},
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub struct Launcher {
    catalog: RwLock<Option<Arc<Catalog>>>,
    pub shortcut: Mutex<String>,
    pub store: Arc<Store>,
    pub pause: Arc<PauseController>,
    pub sync: Arc<SyncWorker>,
}

impl Default for Launcher {
    fn default() -> Self {
        let db_path = gio::glib::user_data_dir().join("fluxcast/fluxcast.db");
        let store = Store::open(&db_path)
            .or_else(|_| Store::open_in_memory())
            .expect("No se pudo inicializar SQLite");
        let _ = store.seed_default_snippets();
        Self {
            catalog: RwLock::new(None),
            shortcut: Mutex::new(String::new()),
            store: Arc::new(store),
            pause: Arc::new(PauseController::default()),
            sync: Arc::new(SyncWorker::default()),
        }
    }
}

impl Launcher {
    fn catalog(&self) -> Arc<Catalog> {
        if let Some(catalog) = self.catalog.read().unwrap().as_ref() {
            return Arc::clone(catalog);
        }
        self.refresh()
    }

    pub fn refresh(&self) -> Arc<Catalog> {
        let catalog = Arc::new(system_catalog());
        *self.catalog.write().unwrap() = Some(Arc::clone(&catalog));
        catalog
    }
}

pub fn warm_catalog(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<Launcher>();
        let _ = state.catalog();
    });
}

#[derive(Serialize)]
pub struct Hit {
    id: usize,
    name: String,
    detail: String,
    kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet_id: Option<String>,
}

#[derive(Serialize)]
pub struct Results {
    hits: Vec<Hit>,
    utility: Option<Output>,
    problem: Option<&'static str>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    All,
    App,
    Project,
    Tools,
}

fn action_detail(id: &str) -> &'static str {
    match id {
        "plasma:lock" => "Bloquea la sesión actual de Plasma (loginctl)",
        "plasma:suspend" => "Suspende el equipo (systemctl suspend)",
        "plasma:copy-local-ip" => "Copia la dirección IPv4 local al portapapeles",
        "plasma:copy-tailscale-ip" => "Copia la dirección IPv4 de Tailscale al portapapeles",
        "plasma:toggle-clipboard-pause" => {
            "Alterna el modo pausa / incógnito del historial de portapapeles"
        }
        _ => "Acción rápida de Plasma",
    }
}

#[tauri::command]
pub async fn refresh_catalog(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app.state::<Launcher>().refresh();
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search(app: AppHandle, query: String, scope: Scope) -> Result<Results, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Launcher>();
        let trimmed = query.trim();

        if matches!(scope, Scope::All | Scope::Tools) && trimmed.starts_with('!') {
            let snippet_query = trimmed.trim_start_matches('!');
            let snippets = state
                .store
                .list_snippets(
                    if snippet_query.is_empty() {
                        ""
                    } else {
                        trimmed
                    },
                    None,
                )
                .unwrap_or_default();
            let hits = snippets
                .into_iter()
                .enumerate()
                .map(|(idx, s)| Hit {
                    id: 100_000 + idx,
                    name: match &s.keyword {
                        Some(kw) => format!("{} ({kw})", s.title),
                        None => s.title,
                    },
                    detail: s.body,
                    kind: Kind::Snippet,
                    snippet_id: Some(s.id),
                })
                .collect();
            return Results {
                hits,
                utility: None,
                problem: None,
            };
        }

        let kind = match scope {
            Scope::App => Some(Kind::App),
            Scope::Project => Some(Kind::Project),
            Scope::Tools => Some(Kind::Action),
            Scope::All => None,
        };
        let utility = matches!(scope, Scope::All | Scope::Tools)
            .then(|| evaluate(&query))
            .flatten();
        let (utility, problem) = match utility {
            Some(Ok(result)) => (Some(result), None),
            Some(Err(problem)) => (None, Some(problem)),
            None => (None, None),
        };
        if utility.is_some()
            || problem.is_some()
            || (matches!(scope, Scope::Tools) && trimmed.is_empty())
        {
            return Results {
                hits: Vec::new(),
                utility,
                problem,
            };
        }
        let catalog = state.catalog();
        let hits = catalog
            .search(trimmed, kind, 32)
            .into_iter()
            .filter_map(|id| {
                let entry = catalog.get(id)?;
                if trimmed.is_empty() && entry.kind == Kind::Action {
                    return None;
                }
                let detail = if entry.kind == Kind::Action {
                    action_detail(entry.path.to_str().unwrap_or_default()).to_owned()
                } else {
                    entry.path.display().to_string()
                };
                Some(Hit {
                    id,
                    name: entry.name.clone(),
                    kind: entry.kind,
                    detail,
                    snippet_id: None,
                })
            })
            .collect();
        Results {
            hits,
            utility,
            problem,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Opener {
    Dolphin,
    Code,
    Konsole,
    Neovim,
}

fn project_command(path: &Path, opener: Opener) -> Command {
    let program = match opener {
        Opener::Dolphin => "dolphin",
        Opener::Code => "code",
        _ => "konsole",
    };
    let mut cmd = Command::new(program);
    cmd.current_dir(path);
    match opener {
        Opener::Dolphin | Opener::Code => {
            cmd.arg(path);
        }
        Opener::Konsole => {
            cmd.arg("--workdir").arg(path);
        }
        Opener::Neovim => {
            cmd.arg("--workdir").arg(path).args(["-e", "nvim", "."]);
        }
    }
    cmd
}

fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

fn parse_ip_addr_output(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let cidr = line.split_whitespace().nth(3)?;
        let ip = cidr.split('/').next()?;
        ip.parse::<IpAddr>()
            .ok()
            .filter(|addr| !addr.is_loopback())
            .map(|addr| addr.to_string())
    })
}

fn local_ipv4() -> Result<String, String> {
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("1.1.1.1:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                if !addr.ip().is_loopback() {
                    return Ok(addr.ip().to_string());
                }
            }
        }
    }
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output()
        .map_err(|e| format!("No se pudo consultar la IP local: {e}"))?;
    if !output.status.success() {
        return Err("No se encontró una dirección IP local activa".into());
    }
    parse_ip_addr_output(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "No se encontró una dirección IP local activa".into())
}

fn tailscale_ipv4() -> Result<String, String> {
    let output = Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .map_err(|e| format!("Tailscale no está disponible: {e}"))?;
    if !output.status.success() {
        return Err("Tailscale no está conectado".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Tailscale no devolvió ninguna IPv4".into())
}

fn set_system_clipboard(state: &Launcher, value: &str) {
    state.pause.mark_internal_copy(value);
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_text(value);
    if !state.pause.is_paused() {
        let _ = state.store.record_clipboard(value);
    }
}

fn current_clipboard_text(state: &Launcher) -> String {
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)
        .wait_for_text()
        .map(|s| s.to_string())
        .or_else(|| state.store.latest_clipboard_text().ok().flatten())
        .unwrap_or_default()
}

fn run_action(app: &AppHandle, state: &Launcher, action: &str) -> Result<(), String> {
    match action {
        "plasma:lock" => Command::new("loginctl")
            .arg("lock-session")
            .spawn()
            .map(reap)
            .map_err(|e| format!("No se pudo bloquear la sesión: {e}")),
        "plasma:suspend" => Command::new("systemctl")
            .arg("suspend")
            .spawn()
            .map(reap)
            .map_err(|e| format!("No se pudo suspender el equipo: {e}")),
        "plasma:copy-local-ip" => {
            let ip = local_ipv4()?;
            set_system_clipboard(state, &ip);
            let _ = app.emit("clipboard-updated", ());
            Ok(())
        }
        "plasma:copy-tailscale-ip" => {
            let ip = tailscale_ipv4()?;
            set_system_clipboard(state, &ip);
            let _ = app.emit("clipboard-updated", ());
            Ok(())
        }
        "plasma:toggle-clipboard-pause" => {
            let status = state.pause.toggle_manual();
            let _ = app.emit("pause-updated", &status);
            Ok(())
        }
        _ => Err("Acción desconocida".into()),
    }
}

#[tauri::command]
pub fn open_result(
    app: AppHandle,
    state: State<'_, Launcher>,
    id: usize,
    opener: Opener,
) -> Result<(), String> {
    let catalog = state
        .catalog
        .read()
        .unwrap()
        .clone()
        .ok_or("Catálogo no inicializado")?;
    let entry = catalog.get(id).ok_or("Resultado desconocido")?;
    match entry.kind {
        Kind::App => gio::DesktopAppInfo::from_filename(&entry.path)
            .ok_or("La aplicación ya no está disponible")?
            .launch(&[], None::<&gio::AppLaunchContext>)
            .map_err(|e| e.to_string()),
        Kind::Project => project_command(&entry.path, opener)
            .spawn()
            .map(reap)
            .map_err(|e| format!("No se pudo abrir con {opener:?}: {e}")),
        Kind::Action => run_action(&app, &state, entry.path.to_str().unwrap_or_default()),
        Kind::Snippet => Err("Usa use_snippet para expandir snippets".into()),
    }
}

#[tauri::command]
pub fn copy_value(app: AppHandle, state: State<'_, Launcher>, value: String) {
    set_system_clipboard(&state, &value);
    let _ = app.emit("clipboard-updated", ());
}

#[tauri::command]
pub fn hide_launcher(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn shortcut_status(state: State<'_, Launcher>) -> String {
    state.shortcut.lock().unwrap().clone()
}

#[tauri::command]
pub async fn list_clipboard(
    app: AppHandle,
    query: String,
    pinned_only: bool,
) -> Result<Vec<ClipboardItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Launcher>()
            .store
            .list_clipboard(&query, pinned_only, 100)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn copy_clipboard_item(
    app: AppHandle,
    state: State<'_, Launcher>,
    id: String,
    plain: bool,
) -> Result<String, String> {
    let item = state
        .store
        .get_clipboard_item(&id)?
        .ok_or_else(|| "Elemento de portapapeles no encontrado".to_string())?;
    let text = if plain {
        plain_text(&item.content)
    } else {
        item.content
    };
    set_system_clipboard(&state, &text);
    let _ = app.emit("clipboard-updated", ());
    Ok(text)
}

#[tauri::command]
pub fn toggle_pin_clipboard(
    app: AppHandle,
    state: State<'_, Launcher>,
    id: String,
) -> Result<bool, String> {
    let pinned = state.store.toggle_pin_clipboard(&id)?;
    let _ = app.emit("clipboard-updated", ());
    Ok(pinned)
}

#[tauri::command]
pub fn delete_clipboard_item(
    app: AppHandle,
    state: State<'_, Launcher>,
    id: String,
) -> Result<(), String> {
    state.store.delete_clipboard_item(&id)?;
    let _ = app.emit("clipboard-updated", ());
    Ok(())
}

#[tauri::command]
pub fn clear_clipboard_history(
    app: AppHandle,
    state: State<'_, Launcher>,
) -> Result<usize, String> {
    let removed = state.store.clear_unpinned_clipboard()?;
    let _ = app.emit("clipboard-updated", ());
    Ok(removed)
}

#[tauri::command]
pub fn clipboard_pause_status(state: State<'_, Launcher>) -> PauseStatus {
    state.pause.status()
}

#[tauri::command]
pub fn set_clipboard_pause(
    app: AppHandle,
    state: State<'_, Launcher>,
    minutes: Option<u64>,
    manual: bool,
) -> PauseStatus {
    let status = state.pause.set_pause(minutes, manual);
    let _ = app.emit("pause-updated", &status);
    status
}

#[tauri::command]
pub async fn list_snippets(
    app: AppHandle,
    query: String,
    tag: Option<String>,
) -> Result<Vec<Snippet>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Launcher>()
            .store
            .list_snippets(&query, tag.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn save_snippet(state: State<'_, Launcher>, input: SnippetInput) -> Result<Snippet, String> {
    state.store.save_snippet(input)
}

#[tauri::command]
pub fn use_snippet(
    app: AppHandle,
    state: State<'_, Launcher>,
    id: String,
) -> Result<String, String> {
    let current_clip = current_clipboard_text(&state);
    let expanded = state.store.use_snippet(&id, &current_clip)?;
    set_system_clipboard(&state, &expanded);
    let _ = app.emit("clipboard-updated", ());
    Ok(expanded)
}

#[tauri::command]
pub fn delete_snippet(state: State<'_, Launcher>, id: String) -> Result<(), String> {
    state.store.delete_snippet(&id)
}

#[tauri::command]
pub async fn sync_status(state: State<'_, Launcher>) -> Result<SyncStatus, String> {
    state.sync.status(&state.store).await
}

#[tauri::command]
pub async fn set_sync_server_url(
    state: State<'_, Launcher>,
    server_url: String,
) -> Result<SyncStatus, String> {
    let _ = state.sync.set_server_url(&server_url)?;
    state.sync.status(&state.store).await
}

#[tauri::command]
pub async fn rotate_sync_passphrase(
    state: State<'_, Launcher>,
    passphrase: String,
) -> Result<SyncStatus, String> {
    state
        .sync
        .rotate_passphrase(&state.store, &passphrase)
        .await
}

#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    state: State<'_, Launcher>,
) -> Result<SyncCycleReport, String> {
    state.sync.sync_now(&app, &state.store).await
}

#[tauri::command]
pub async fn generate_pairing_qr(state: State<'_, Launcher>) -> Result<PairingQrView, String> {
    state.sync.create_pairing_qr().await
}

#[tauri::command]
pub async fn list_synced_devices(state: State<'_, Launcher>) -> Result<Vec<DeviceInfo>, String> {
    state.sync.list_devices().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_launchers_keep_paths_in_one_argument() {
        let path = Path::new("/tmp/Proyecto con espacios");
        for (tool, program, args) in [
            (Opener::Code, "code", vec![path.to_str().unwrap()]),
            (Opener::Dolphin, "dolphin", vec![path.to_str().unwrap()]),
            (
                Opener::Konsole,
                "konsole",
                vec!["--workdir", path.to_str().unwrap()],
            ),
            (
                Opener::Neovim,
                "konsole",
                vec!["--workdir", path.to_str().unwrap(), "-e", "nvim", "."],
            ),
        ] {
            let cmd = project_command(path, tool);
            assert_eq!(cmd.get_program(), program);
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), args);
            assert_eq!(cmd.get_current_dir(), Some(path));
        }
    }

    #[test]
    fn parses_first_non_loopback_ipv4_from_ip_output() {
        let sample = "1: lo    inet 127.0.0.1/8 scope host lo\n2: wlan0    inet 192.168.1.42/24 brd 192.168.1.255 scope global dynamic noprefixroute wlan0\n";
        assert_eq!(
            parse_ip_addr_output(sample).as_deref(),
            Some("192.168.1.42")
        );
        assert_eq!(parse_ip_addr_output("1: lo inet 127.0.0.1/8"), None);
    }
}
