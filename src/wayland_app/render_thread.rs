use std::sync::mpsc::{Receiver, Sender};

use crate::wayland_app::graphics::Graphics;
use crate::wayland_app::AppConfiguration;
use wayland_client::protocol::{wl_display, wl_surface};

#[derive(Debug)]
pub enum RenderCommand {
    Render { elapsed: f32 },
    Resize { width: u32, height: u32 },
    Exit,
}

#[derive(Debug)]
pub enum RenderEvent {
    FrameComplete,
}

pub fn run_render_thread(
    display: wl_display::WlDisplay,
    surface: wl_surface::WlSurface,
    width: u32,
    height: u32,
    conf: AppConfiguration,
    rx: Receiver<RenderCommand>,
    tx: Sender<RenderEvent>,
) {
    let mut graphics = Graphics::new(&display, &surface, width, height, &conf);

    // Mark idle initially
    let _ = tx.send(RenderEvent::FrameComplete);

    while let Ok(cmd) = rx.recv() {
        match cmd {
            RenderCommand::Render { elapsed } => {
                graphics.render(elapsed);
                let _ = tx.send(RenderEvent::FrameComplete);
            }
            RenderCommand::Resize { width, height } => {
                graphics.resize(width, height);
            }
            RenderCommand::Exit => break,
        }
    }
}