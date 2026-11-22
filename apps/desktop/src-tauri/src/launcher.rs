use fluxcast_core::{
    catalog::{Catalog, Kind},
    desktop::system_catalog,
    utilities::{evaluate, Output},
};
use gio::prelude::AppInfoExt;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, UdpSocket},
    path::Path,
    process::{Child, Command},
    sync::{Arc, Mutex, RwLock},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

#[derive(Default)]
pub struct Launcher {
    catalog: RwLock<Option<Arc<Catalog>>>,
    pub shortcut: Mutex<String>,
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
        let trimmed = query.trim();
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

fn run_action(action: &str) -> Result<(), String> {
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
            copy_value(ip);
            Ok(())
        }
        "plasma:copy-tailscale-ip" => {
            let ip = tailscale_ipv4()?;
            copy_value(ip);
            Ok(())
        }
        _ => Err("Acción desconocida".into()),
    }
}

#[tauri::command]
pub fn open_result(state: State<'_, Launcher>, id: usize, opener: Opener) -> Result<(), String> {
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
        Kind::Action => run_action(entry.path.to_str().unwrap_or_default()),
    }
}

#[tauri::command]
pub fn copy_value(value: String) {
    gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_text(&value);
}

#[tauri::command]
pub fn hide_launcher(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn shortcut_status(state: State<'_, Launcher>) -> String {
    state.shortcut.lock().unwrap().clone()
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
