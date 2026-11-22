use super::{should_ignore_offer, ClipboardProvider, KDE_PASSWORD_HINT_MIME};
use std::{process::Command, thread, time::Duration};

pub struct FallbackClipboardProvider;

impl ClipboardProvider for FallbackClipboardProvider {
    fn name(&self) -> &'static str {
        "x11-wl-paste-fallback"
    }

    fn run(
        &self,
        is_paused: Box<dyn Fn() -> bool + Send + Sync>,
        on_text: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<(), String> {
        let mut last_seen = String::new();
        loop {
            thread::sleep(Duration::from_millis(650));
            if is_paused() {
                continue;
            }
            let Some((mimes, hint, text)) = probe_system_clipboard() else {
                continue;
            };
            if should_ignore_offer(&mimes, hint.as_deref(), Some(&text)) {
                continue;
            }
            if text != last_seen {
                last_seen = text.clone();
                on_text(text);
            }
        }
    }
}

fn probe_system_clipboard() -> Option<(Vec<String>, Option<Vec<u8>>, String)> {
    let types_out = Command::new("wl-paste")
        .arg("--list-types")
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let mimes: Vec<String> = String::from_utf8_lossy(&types_out.stdout)
        .lines()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let hint = if mimes
        .iter()
        .any(|m| m.eq_ignore_ascii_case(KDE_PASSWORD_HINT_MIME))
    {
        Command::new("wl-paste")
            .args(["--no-newline", "--type", KDE_PASSWORD_HINT_MIME])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
    } else {
        None
    };
    if should_ignore_offer(&mimes, hint.as_deref(), None) {
        return None;
    }
    let text_out = Command::new("wl-paste")
        .arg("--no-newline")
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(text_out.stdout).ok()?;
    Some((mimes, hint, text))
}
