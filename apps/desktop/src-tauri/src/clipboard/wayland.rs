use super::{
    has_password_manager_mime, preferred_text_mime, should_ignore_offer, ClipboardProvider,
};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    os::fd::{AsFd, FromRawFd, OwnedFd},
    time::Instant,
};
use wayland_client::{
    backend::ObjectId,
    event_created_child,
    protocol::{wl_registry, wl_seat::WlSeat},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

const MAX_CLIPBOARD_BYTES: usize = 512 * 1024;
const PIPE_TIMEOUT_MS: i32 = 600;

pub struct WaylandDataControlProvider;

impl ClipboardProvider for WaylandDataControlProvider {
    fn name(&self) -> &'static str {
        "wayland-data-control"
    }

    fn run(
        &self,
        is_paused: Box<dyn Fn() -> bool + Send + Sync>,
        on_text: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<(), String> {
        let conn = Connection::connect_to_env()
            .map_err(|e| format!("No se pudo conectar al socket Wayland: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());

        let mut state = WaylandState {
            conn: conn.clone(),
            seat: None,
            ext_manager: None,
            wlr_manager: None,
            offers: HashMap::new(),
            is_paused,
            on_text,
        };

        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Fallo en roundtrip inicial de Wayland: {e}"))?;

        let seat = state
            .seat
            .clone()
            .ok_or_else(|| "Wayland no expuso ningún wl_seat".to_string())?;

        if let Some(ext_mgr) = &state.ext_manager {
            let _device = ext_mgr.get_data_device(&seat, &qh, ());
        } else if let Some(wlr_mgr) = &state.wlr_manager {
            let _device = wlr_mgr.get_data_device(&seat, &qh, ());
        } else {
            return Err(
                "El compositor no implementa ext_data_control_manager_v1 ni zwlr_data_control_manager_v1"
                    .into(),
            );
        }

        loop {
            queue
                .blocking_dispatch(&mut state)
                .map_err(|e| format!("Desconexión del bucle Wayland: {e}"))?;
        }
    }
}

struct WaylandState {
    conn: Connection,
    seat: Option<WlSeat>,
    ext_manager: Option<ExtDataControlManagerV1>,
    wlr_manager: Option<ZwlrDataControlManagerV1>,
    offers: HashMap<ObjectId, Vec<String>>,
    is_paused: Box<dyn Fn() -> bool + Send + Sync>,
    on_text: Box<dyn Fn(String) + Send + Sync>,
}

impl WaylandState {
    fn handle_selection<F>(&mut self, id: ObjectId, mut receive_fn: F)
    where
        F: FnMut(String, OwnedFd),
    {
        let mimes = self.offers.remove(&id).unwrap_or_default();
        if (self.is_paused)() {
            return;
        }
        if let Some(pw_mime) = has_password_manager_mime(&mimes) {
            let hint = read_pipe_from_compositor(&self.conn, |fd| receive_fn(pw_mime, fd), 256);
            if should_ignore_offer(&mimes, hint.as_deref(), None) {
                return;
            }
        } else if should_ignore_offer(&mimes, None, None) {
            return;
        }

        let Some(text_mime) = preferred_text_mime(&mimes) else {
            return;
        };
        let Some(bytes) = read_pipe_from_compositor(
            &self.conn,
            |fd| receive_fn(text_mime, fd),
            MAX_CLIPBOARD_BYTES,
        ) else {
            return;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return;
        };
        if !should_ignore_offer(&mimes, None, Some(&text)) {
            (self.on_text)(text);
        }
    }
}

fn read_pipe_from_compositor<F>(conn: &Connection, request: F, max_bytes: usize) -> Option<Vec<u8>>
where
    F: FnOnce(OwnedFd),
{
    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if rc != 0 {
        return None;
    }
    let read_raw = fds[0];
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    let mut read_file = unsafe { File::from_raw_fd(read_raw) };

    request(write_fd);
    let _ = conn.flush();

    let start = Instant::now();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let elapsed = start.elapsed().as_millis() as i32;
        let remaining = (PIPE_TIMEOUT_MS - elapsed).max(0);
        let mut pfd = libc::pollfd {
            fd: read_raw,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let poll_rc = unsafe { libc::poll(&mut pfd, 1, remaining) };
        if poll_rc <= 0 {
            return (!buffer.is_empty()).then_some(buffer);
        }
        match read_file.read(&mut chunk) {
            Ok(0) => return Some(buffer),
            Ok(n) => {
                let take = n.min(max_bytes.saturating_sub(buffer.len()));
                buffer.extend_from_slice(&chunk[..take]);
                if buffer.len() >= max_bytes {
                    return Some(buffer);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if (pfd.revents & libc::POLLHUP) != 0 {
                    return Some(buffer);
                }
            }
            Err(_) => return None,
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                "ext_data_control_manager_v1" if state.ext_manager.is_none() => {
                    state.ext_manager =
                        Some(registry.bind::<ExtDataControlManagerV1, _, _>(name, 1, qh, ()));
                }
                "zwlr_data_control_manager_v1" if state.wlr_manager.is_none() => {
                    state.wlr_manager = Some(registry.bind::<ZwlrDataControlManagerV1, _, _>(
                        name,
                        version.min(2),
                        qh,
                        (),
                    ));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            ext_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                let obj_id = offer.id();
                state.handle_selection(obj_id, |mime, fd| {
                    offer.receive(mime, fd.as_fd());
                });
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(WaylandState, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                let obj_id = offer.id();
                state.handle_selection(obj_id, |mime, fd| {
                    offer.receive(mime, fd.as_fd());
                });
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(WaylandState, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}
