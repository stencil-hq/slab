//! `slab-native FILE.slab --port N` — mount the live window kernel as an SDP
//! session (N13/C-41). A background TCP listener forwards NDJSON request
//! lines through the winit user-event channel; the [`ShellHost`] drains each
//! one against the window's own [`NativeDocument`] with a
//! [`slab_drive::RequestPump`], so automation drives exactly the instance the
//! window paints. Responses flow back over the socket in request order.
//!
//! One client at a time: a second connection receives the single normative
//! `session busy` error line (SDP §1, via [`slab_drive::reject_busy`]) and is
//! closed immediately.
//! `doc.reload`/`doc.load` stay denied on a window mount — the renderer's
//! image and font resources are registered once at startup — and
//! `protocol.quit` closes the window with exit status 0.

use crate::NativeDocument;
use crate::view::{NativeShell, ShellEvent, ShellHost, ShellOptions, install_sigterm};
use slab_drive::RequestPump;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::Window;

/// One NDJSON request line forwarded from the socket thread to the loop.
pub struct SdpRequest {
    line: String,
    reply: mpsc::Sender<String>,
}

/// Host policy that applies SDP requests to the window's live kernel.
struct SdpHost {
    pump: RequestPump,
}

impl ShellHost<SdpRequest> for SdpHost {
    fn user_event(
        &mut self,
        document: &mut NativeDocument,
        _window: &Window,
        event_loop: &ActiveEventLoop,
        event: SdpRequest,
    ) -> bool {
        let result = self.pump.request(&mut document.inst, &event.line);
        for effects in &result.effects {
            // the same signal path a windowed interaction takes
            self.effects(document, effects);
        }
        // receiver gone = client disconnected mid-request; nothing to answer
        let _ = event.reply.send(result.response.to_string());
        if result.quit {
            event_loop.exit();
        }
        true
    }
}

/// Runs the windowed viewer with an SDP listener sharing its kernel.
pub(crate) fn run_window(
    doc: NativeDocument,
    slir: slab_slir::Slir,
    doc_path: PathBuf,
    options: ShellOptions,
    port: u16,
) -> Result<(), String> {
    let event_loop = EventLoop::<ShellEvent<SdpRequest>>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    install_sigterm(event_loop.create_proxy());
    let bound = spawn_listener(port, event_loop.create_proxy())?;
    eprintln!("slab-native: SDP session on 127.0.0.1:{bound}");
    let pump = RequestPump::new(doc_path, slir, doc.imgs.clone());
    let mut app = NativeShell::new(doc, options, event_loop.create_proxy(), SdpHost { pump });
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    eprintln!("slab-native: presented {} frames", app.frames);
    if app.frames == 0 {
        return Err("no frames presented".into());
    }
    Ok(())
}

/// Binds the loopback listener and serves connections on a background thread.
/// Returns the bound port (`--port 0` picks a free one).
fn spawn_listener(port: u16, proxy: EventLoopProxy<ShellEvent<SdpRequest>>) -> Result<u16, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("--port {port}: {e}"))?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("--port {port}: {e}"))?
        .port();
    let active = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                slab_drive::reject_busy(stream);
                continue;
            }
            let proxy = proxy.clone();
            let active = Arc::clone(&active);
            std::thread::spawn(move || {
                serve_connection(stream, &proxy);
                active.store(false, Ordering::Release);
            });
        }
    });
    Ok(bound)
}

/// Pumps one client's NDJSON lines through the event loop until it hangs up
/// (or the event loop itself is gone).
fn serve_connection(stream: TcpStream, proxy: &EventLoopProxy<ShellEvent<SdpRequest>>) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let (reply, responses) = mpsc::channel::<String>();
    let mut write_half = stream;
    let writer = std::thread::spawn(move || {
        for response in responses {
            if write_half
                .write_all(response.as_bytes())
                .and_then(|()| write_half.write_all(b"\n"))
                .is_err()
            {
                break;
            }
        }
    });
    for line in BufReader::new(read_half).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request = SdpRequest {
            line,
            reply: reply.clone(),
        };
        if proxy.send_event(ShellEvent::User(request)).is_err() {
            break;
        }
    }
    drop(reply);
    let _ = writer.join();
}
