use crate::catalog::{Catalog, Entry, Kind};
use gio::{prelude::AppInfoExt, DesktopAppInfo};
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

const MAX_PROJECT_DEPTH: usize = 5;

/// Respeta la precedencia de XDG e incluye exportaciones Flatpak incluso si el
/// proceso se inició antes de que la sesión actualizara XDG_DATA_DIRS.
pub fn system_catalog() -> Catalog {
    let home = gio::glib::home_dir();
    let data = gio::glib::user_data_dir();
    let mut data_dirs = vec![data.clone()];
    data_dirs.extend(gio::glib::system_data_dirs());
    data_dirs.extend([
        data.join("flatpak/exports/share"),
        "/var/lib/flatpak/exports/share".into(),
    ]);
    let apps: Vec<_> = data_dirs.iter().map(|p| p.join("applications")).collect();
    let roots = env::var_os("FLUXCAST_PROJECT_DIRS")
        .map(|value| {
            env::split_paths(&value)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_else(|| {
            ["Proyectos", "Projects", "Dev"]
                .map(|p| home.join(p))
                .to_vec()
        });
    let mut entries = collect_entries(&apps, &roots);
    entries.extend(plasma_actions());
    Catalog::new(entries)
}

pub fn plasma_actions() -> Vec<Entry> {
    [
        (
            "Bloquear sesión",
            "plasma:lock",
            "lock screen pantalla bloquear sesion loginctl plasma kde",
        ),
        (
            "Suspender equipo",
            "plasma:suspend",
            "suspend sleep dormir suspender equipo systemctl plasma kde",
        ),
        (
            "Copiar IP local",
            "plasma:copy-local-ip",
            "ip local lan red network ipv4 copiar",
        ),
        (
            "Copiar IP Tailscale",
            "plasma:copy-tailscale-ip",
            "ip tailscale vpn mesh ipv4 copiar",
        ),
    ]
    .into_iter()
    .map(|(name, id, keywords)| {
        Entry::with_keywords(
            name.into(),
            Kind::Action,
            PathBuf::from(id),
            keywords.into(),
        )
    })
    .collect()
}

pub fn discover(app_dirs: &[PathBuf], project_dirs: &[PathBuf]) -> Catalog {
    Catalog::new(collect_entries(app_dirs, project_dirs))
}

fn collect_entries(app_dirs: &[PathBuf], project_dirs: &[PathBuf]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut desktop_ids = HashSet::new();
    for dir in app_dirs {
        for file in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
            let path = file.path();
            if path.extension().is_none_or(|ext| ext != "desktop") || !path.is_file() {
                continue;
            }
            let id = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('/', "-");
            if !desktop_ids.insert(id.clone()) {
                continue;
            }
            if let Some(app) = DesktopAppInfo::from_filename(path) {
                if !app.is_hidden() && app.should_show() {
                    let mut tokens = Vec::new();
                    if let Some(generic) = app.generic_name() {
                        tokens.push(generic.to_string());
                    }
                    tokens.extend(app.keywords().into_iter().map(|k| k.to_string()));
                    tokens.push(id.trim_end_matches(".desktop").to_owned());
                    entries.push(Entry::with_keywords(
                        app.display_name().into(),
                        Kind::App,
                        path.to_owned(),
                        tokens.join(" "),
                    ));
                }
            }
        }
    }
    let mut projects = HashSet::new();
    for root in project_dirs {
        let mut tree = WalkDir::new(root).max_depth(MAX_PROJECT_DEPTH).into_iter();
        while let Some(item) = tree.next() {
            let Ok(dir) = item else { continue };
            let path = dir.path();
            if !dir.file_type().is_dir() {
                continue;
            }
            let name = dir.file_name().to_string_lossy();
            if dir.depth() > 0
                && (name.starts_with('.')
                    || matches!(
                        name.as_ref(),
                        "node_modules"
                            | "target"
                            | "build"
                            | "dist"
                            | "vendor"
                            | "__pycache__"
                            | "venv"
                            | "out"
                    ))
            {
                tree.skip_current_dir();
                continue;
            }
            let marker = path.join(".git");
            if marker.is_dir() || is_worktree_marker(&marker) {
                if let Ok(real) = path.canonicalize() {
                    if projects.insert(real.clone()) {
                        entries.push(Entry::new(name.into_owned(), Kind::Project, real));
                    }
                }
                tree.skip_current_dir();
            }
        }
    }
    entries
}

fn is_worktree_marker(path: &Path) -> bool {
    use std::io::Read;
    let mut prefix = [0; 8];
    path.is_file()
        && std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut prefix))
            .is_ok()
        && &prefix == b"gitdir: "
}
