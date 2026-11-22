# FluxCast — Native Fedora KDE Plasma 6 (Wayland) Integration Guide

## 1. Wayland Clipboard Monitoring (`ext-data-control-v1`)

Unlike legacy X11 clipboard tools that poll or require XWayland focus hacks, FluxCast implements `WaylandDataControlProvider` (`apps/desktop/src-tauri/src/clipboard/wayland.rs`) using official Wayland staging and wlroots protocols:

1. **Primary protocol (KWin / KDE Plasma 6):** `ext_data_control_manager_v1` (`ext-data-control-v1`).
2. **Automatic Wayland fallback:** `zwlr_data_control_manager_v1` (`wlr-data-control-unstable-v1`).
3. **Secondary fallback:** `FallbackClipboardProvider` (`wl-paste`).

### Strict Password Manager Exclusion (`x-kde-passwordManagerHint=secret`)

Password managers on KDE Plasma (KeePassXC, Bitwarden, 1Password, KWallet) attach the MIME type `x-kde-passwordManagerHint` with value `secret` to sensitive clipboard selections.

When `WaylandDataControlProvider` receives a `Selection` event:
1. Before requesting `text/plain`, it inspects the advertised MIME list.
2. If `x-kde-passwordManagerHint`, `x-gnome-passwordManagerHint`, or `application/x-kde-password` is advertised, FluxCast reads the hint pipe first.
3. If the hint equals `secret` (or if reading the hint fails — **fail-closed**), FluxCast destroys the offer immediately without ever reading `text/plain`, persisting to SQLite, or queuing to `sync_outbox`.

---

## 2. Global Shortcuts via `xdg-desktop-portal-kde` + UNIX Socket

- **Portal integration (`apps/desktop/src-tauri/src/shortcuts.rs`):**
  Uses `ashpd::desktop::global_shortcuts::GlobalShortcuts` to register `toggle_launcher` with preferred trigger `ALT+space` and subscribes to both `Activated` and `ShortcutsChanged` D-Bus signals. Users can customize the shortcut anytime in *KDE System Settings → Shortcuts*.
- **Sub-3ms UNIX Socket (`apps/desktop/src-tauri/src/ipc_socket.rs` & `cli.rs`):**
  The daemon listens on `$XDG_RUNTIME_DIR/fluxcast.sock` (`0600` permissions) protected by `$XDG_RUNTIME_DIR/fluxcast.lock`. Running `fluxcast toggle` (or `fluxcast` when already active) toggles window visibility in **~2.2 ms**.

---

## 3. Master Key Storage in `ksecretd` / KWallet (`oo7`)

`apps/desktop/src-tauri/src/keyring.rs` connects to the Freedesktop Secret Service (`org.freedesktop.secrets`, provided by `ksecretd` in Plasma 6) via the `oo7` crate and stores the Base64URL-encoded 256-bit Master Key under attributes:
- `application = dev.fluxcast.desktop`
- `kind = e2ee_master_key`

---

## 4. User Service (`systemd --user`)

To install FluxCast and enable autostart with your KDE Plasma session:

```sh
./scripts/install-fedora-kde.sh
```

Check status or logs:
```sh
systemctl --user status fluxcast.service
journalctl --user -u fluxcast.service -f
```
