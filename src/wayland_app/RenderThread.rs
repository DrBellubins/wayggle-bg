use std::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub enum RenderCommand {
    Render { elapsed: f32 },
    Resize { width: u32, height: u32 },
    Exit,
}

#[derive(Debug)]
pub enum RenderEvent {
    FrameComplete { elapsed: f32 },
}

use crate::wayland_app::graphics::Graphics;
use crate::wayland_app::AppConfiguration;
use wayland_client::protocol::{wl_display, wl_surface};

pub fn run_render_thread(
    display: wl_display::WlDisplay,
    surface: wl_surface::WlSurface,
    width: u32,
    height: u32,
    conf: AppConfiguration,
    rx: Receiver<RenderCommand>,
    tx: Sender<RenderEvent>,
) {
    // Create GL/EGL on this thread
    let mut graphics = Graphics::new(&display, &surface, width, height, &conf);

    // Optional: render one initial frame so you see something immediately
    // (You can also trigger this via a RenderCommand from the Wayland thread.)
    let _ = tx.send(RenderEvent::FrameComplete { elapsed: 0.0 });

    while let Ok(cmd) = rx.recv() {
        match cmd {
            RenderCommand::Render { elapsed } => {
                graphics.render(elapsed);
                let _ = tx.send(RenderEvent::FrameComplete { elapsed });
            }
            RenderCommand::Resize { width, height } => {
                graphics.resize(width, height);
            }
            RenderCommand::Exit => break,
        }
    }
}