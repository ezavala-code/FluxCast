//! Catálogo local, búsqueda, portapapeles, snippets y utilidades del launcher.
pub mod catalog;
pub mod classifier;
#[cfg(target_os = "linux")]
pub mod desktop;
pub mod store;
pub mod templates;
pub mod utilities;
