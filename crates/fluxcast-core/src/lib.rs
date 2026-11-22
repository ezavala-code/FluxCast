//! Catálogo local, búsqueda y utilidades del launcher.
pub mod catalog;
#[cfg(target_os = "linux")]
pub mod desktop;
pub mod utilities;
