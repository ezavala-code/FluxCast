use fluxcast_core::{
    catalog::{Catalog, Kind},
    desktop::system_catalog,
    utilities::{evaluate, Output},
};
use gio::prelude::AppInfoExt;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::{Child, Command},
    sync::{Mutex, OnceLock},
};
use tauri::{Manager, State, WebviewWindow};

#[derive(Default)]
pub struct Launcher {
    catalog: OnceLock<Catalog>,
    pub shortcut: Mutex<String>,
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

#[tauri::command]
pub async fn search(app: tauri::AppHandle, query: String, scope: Scope) -> Result<Results, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Launcher>();
        let kind = match scope {
            Scope::App => Some(Kind::App),
            Scope::Project => Some(Kind::Project),
            _ => None,
        };
        let utility = kind.is_none().then(|| evaluate(&query)).flatten();
        let (utility, problem) = match utility {
            Some(Ok(result)) => (Some(result), None),
            Some(Err(problem)) => (None, Some(problem)),
            None => (None, None),
        };
        if utility.is_some() || problem.is_some() || matches!(scope, Scope::Tools) {
            return Results {
                hits: Vec::new(),
                utility,
                problem,
            };
        }
        let catalog = state.catalog.get_or_init(system_catalog);
        let hits = catalog
            .search(query.trim(), kind, 32)
            .into_iter()
            .map(|id| {
                let entry = catalog.get(id).unwrap();
                Hit {
                    id,
                    name: entry.name.clone(),
                    kind: entry.kind,
                    detail: entry.path.display().to_string(),
                }
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

#[tauri::command]
pub fn open_result(state: State<'_, Launcher>, id: usize, opener: Opener) -> Result<(), String> {
    let entry = state
        .catalog
        .get()
        .and_then(|catalog| catalog.get(id))
        .ok_or("Resultado desconocido")?;
    match entry.kind {
        Kind::App => gio::DesktopAppInfo::from_filename(&entry.path)
            .ok_or("La aplicación ya no está disponible")?
            .launch(&[], None::<&gio::AppLaunchContext>)
            .map_err(|e| e.to_string()),
        Kind::Project => project_command(&entry.path, opener)
            .spawn()
            .map(reap)
            .map_err(|e| format!("No se pudo abrir con {opener:?}: {e}")),
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
}
