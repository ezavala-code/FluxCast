pub mod wayland;
pub mod x11;

use fluxcast_core::{classifier::dedup_hmac, store::Store};
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

pub const KDE_PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";
pub const GNOME_PASSWORD_HINT_MIME: &str = "x-gnome-passwordManagerHint";

const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

pub trait ClipboardProvider: Send {
    fn name(&self) -> &'static str;
    fn run(
        &self,
        is_paused: Box<dyn Fn() -> bool + Send + Sync>,
        on_text: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<(), String>;
}

/// Elige el mejor tipo MIME de texto disponible en una oferta del portapapeles.
pub fn preferred_text_mime(mimes: &[String]) -> Option<String> {
    for &candidate in TEXT_MIMES {
        if let Some(found) = mimes.iter().find(|m| m.as_str() == candidate) {
            return Some(found.clone());
        }
    }
    mimes.iter().find(|m| m.starts_with("text/plain")).cloned()
}

/// Comprueba si la oferta anuncia la marca de gestor de contraseñas de KDE/Klipper.
pub fn has_password_manager_mime(mimes: &[String]) -> Option<String> {
    mimes
        .iter()
        .find(|m| {
            m.eq_ignore_ascii_case(KDE_PASSWORD_HINT_MIME)
                || m.eq_ignore_ascii_case(GNOME_PASSWORD_HINT_MIME)
                || m.eq_ignore_ascii_case("application/x-kde-password")
        })
        .cloned()
}

/// Filtra estrictamente cualquier selección marcada como `secret` por KeePassXC,
/// Bitwarden, 1Password o KWallet (`x-kde-passwordManagerHint=secret`).
/// Si el MIME está presente pero no se pudo leer el valor, falla en cerrado (ignora).
pub fn should_ignore_offer(
    mimes: &[String],
    password_hint: Option<&[u8]>,
    text: Option<&str>,
) -> bool {
    if mimes
        .iter()
        .any(|m| m.eq_ignore_ascii_case("application/x-kde-password"))
    {
        return true;
    }
    if has_password_manager_mime(mimes).is_some() {
        let Some(raw) = password_hint else {
            return true;
        };
        let hint = String::from_utf8_lossy(raw);
        let cleaned = hint.trim().trim_matches('\0');
        if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("secret") {
            return true;
        }
    }
    if let Some(content) = text {
        if content.trim().is_empty() {
            return true;
        }
    }
    false
}

#[derive(Clone, Debug, Default)]
enum PauseMode {
    #[default]
    Active,
    Timed(Instant),
    Manual,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PauseStatus {
    pub paused: bool,
    pub mode: &'static str,
    pub remaining_seconds: Option<u64>,
}

#[derive(Default)]
pub struct PauseController {
    mode: Mutex<PauseMode>,
    last_internal_hmac: Mutex<Option<String>>,
}

impl PauseController {
    pub fn is_paused(&self) -> bool {
        self.status().paused
    }

    pub fn status(&self) -> PauseStatus {
        let mut guard = self.mode.lock().unwrap();
        match *guard {
            PauseMode::Active => PauseStatus {
                paused: false,
                mode: "active",
                remaining_seconds: None,
            },
            PauseMode::Manual => PauseStatus {
                paused: true,
                mode: "manual",
                remaining_seconds: None,
            },
            PauseMode::Timed(deadline) => {
                let now = Instant::now();
                if deadline > now {
                    PauseStatus {
                        paused: true,
                        mode: "timed",
                        remaining_seconds: Some((deadline - now).as_secs().max(1)),
                    }
                } else {
                    *guard = PauseMode::Active;
                    PauseStatus {
                        paused: false,
                        mode: "active",
                        remaining_seconds: None,
                    }
                }
            }
        }
    }

    pub fn set_pause(&self, minutes: Option<u64>, manual: bool) -> PauseStatus {
        {
            let mut guard = self.mode.lock().unwrap();
            *guard = if manual {
                PauseMode::Manual
            } else if let Some(mins) = minutes.filter(|&m| m > 0) {
                PauseMode::Timed(Instant::now() + Duration::from_secs(mins * 60))
            } else {
                PauseMode::Active
            };
        }
        self.status()
    }

    pub fn toggle_manual(&self) -> PauseStatus {
        let currently_paused = self.is_paused();
        self.set_pause(None, !currently_paused)
    }

    pub fn mark_internal_copy(&self, text: &str) {
        *self.last_internal_hmac.lock().unwrap() = Some(dedup_hmac(text));
    }

    pub fn take_if_internal_copy(&self, text: &str) -> bool {
        let hmac = dedup_hmac(text);
        let mut guard = self.last_internal_hmac.lock().unwrap();
        if guard.as_deref() == Some(hmac.as_str()) {
            *guard = None;
            return true;
        }
        false
    }
}

pub fn spawn_monitor(app: AppHandle, store: Arc<Store>, pause: Arc<PauseController>) {
    std::thread::spawn(move || {
        let check_paused = {
            let pause = Arc::clone(&pause);
            move || pause.is_paused()
        };
        let handle_text = {
            let store = Arc::clone(&store);
            let pause = Arc::clone(&pause);
            let app = app.clone();
            move |text: String| {
                if pause.is_paused() || pause.take_if_internal_copy(&text) {
                    return;
                }
                if let Ok(Some(_)) = store.record_clipboard(&text) {
                    let _ = app.emit("clipboard-updated", ());
                }
            }
        };

        let wayland_provider = wayland::WaylandDataControlProvider;
        if let Err(err) = wayland_provider.run(
            Box::new(check_paused.clone()),
            Box::new(handle_text.clone()),
        ) {
            let fallback = x11::FallbackClipboardProvider;
            eprintln!(
                "FluxCast · {} no disponible ({err}); usando {}",
                wayland_provider.name(),
                fallback.name()
            );
            let _ = fallback.run(Box::new(check_paused), Box::new(handle_text));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_kde_password_manager_secret_and_fails_closed() {
        let mimes = vec![
            "text/plain;charset=utf-8".to_owned(),
            KDE_PASSWORD_HINT_MIME.to_owned(),
        ];
        assert!(should_ignore_offer(
            &mimes,
            Some(b"secret"),
            Some("hunter2")
        ));
        assert!(should_ignore_offer(
            &mimes,
            Some(b"SECRET\n"),
            Some("hunter2")
        ));
        // Si el MIME está presente pero la pista no pudo leerse, se ignora por seguridad.
        assert!(should_ignore_offer(&mimes, None, Some("hunter2")));
        assert!(should_ignore_offer(&mimes, Some(b""), Some("hunter2")));
        // Oferta normal sin pista de contraseña se acepta.
        let normal = vec!["text/plain;charset=utf-8".to_owned()];
        assert!(!should_ignore_offer(&normal, None, Some("cargo test")));
        assert!(should_ignore_offer(&normal, None, Some("   ")));
    }

    #[test]
    fn pause_controller_supports_timed_manual_and_toggle_modes() {
        let pause = PauseController::default();
        assert!(!pause.is_paused());

        let timed = pause.set_pause(Some(5), false);
        assert!(timed.paused);
        assert_eq!(timed.mode, "timed");
        assert!(timed.remaining_seconds.unwrap() <= 300);

        let resumed = pause.set_pause(None, false);
        assert!(!resumed.paused);

        let toggled = pause.toggle_manual();
        assert!(toggled.paused);
        assert_eq!(toggled.mode, "manual");
        assert!(!pause.toggle_manual().paused);
    }
}
