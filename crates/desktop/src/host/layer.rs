//! Wayland layer-shell backend: the tray as a borderless surface on the desktop layer.
//!
//! Drawing is egui on wgpu. Idle cost is low because a frame is drawn only after input,
//! a configure event, or when egui asks for one.
use super::{layer_for, Anchor, LayerPlacement};
use eframe::egui;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{self, timer::Timer, EventLoop},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor as WlrAnchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler,
            LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use sq_core::Layer as StackLayer;
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const LAYER_SHELL: &str = "zwlr_layer_shell_v1";

/// True when a Wayland compositor is reachable and offers layer-shell.
pub fn available() -> bool {
    let Ok(conn) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<Probe>(&conn) else {
        return false;
    };
    globals
        .contents()
        .with_list(|list| list.iter().any(|g| g.interface == LAYER_SHELL))
}

struct Probe;
impl
    smithay_client_toolkit::reexports::client::Dispatch<
        smithay_client_toolkit::reexports::client::protocol::wl_registry::WlRegistry,
        smithay_client_toolkit::reexports::client::globals::GlobalListContents,
    > for Probe
{
    fn event(
        _: &mut Self,
        _: &smithay_client_toolkit::reexports::client::protocol::wl_registry::WlRegistry,
        _: smithay_client_toolkit::reexports::client::protocol::wl_registry::Event,
        _: &smithay_client_toolkit::reexports::client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// What the application wants from the surface; the host owns everything else.
pub struct Request {
    pub width: u32,
    pub height: u32,
    pub anchor: Anchor,
    pub margin: i32,
    pub layer: StackLayer,
}

/// How the application wants its surface placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    /// Anchored to a corner with the requested size (the first-run layout).
    Legacy,
    /// Docked at a desktop position, sized to the widgets inside it.
    Free {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Covers the whole output (transparent) so widgets can be dragged anywhere.
    Editing,
}

/// One frame of the application: runs the egui context and returns its output.
pub trait Frame {
    /// Where and how large the application wants its surface, asked after every frame.
    fn surface(&self) -> Surface {
        Surface::Legacy
    }
    /// Desktop position of the docked surface's top-left corner, reported just before the
    /// surface covers the whole output for editing.
    fn docked_origin(&mut self, _origin: (f32, f32)) {}
    /// Called once per drawn frame. Return `false` to close the surface.
    fn update(&mut self, ctx: &egui::Context) -> bool;
    /// The stacking layer the application currently wants.
    fn layer(&self) -> StackLayer;
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
}

struct State {
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    layer: LayerSurface,
    pointer: Option<wl_pointer::WlPointer>,
    gpu: Option<Gpu>,
    adapter: wgpu::Adapter,
    ctx: egui::Context,
    app: Box<dyn Frame>,
    placement: LayerPlacement,
    size: (u32, u32),
    surface: Surface,
    anchor: Anchor,
    margin: i32,
    legacy_size: (u32, u32),
    scale: i32,
    configured: bool,
    events: Vec<egui::Event>,
    pointer_pos: egui::Pos2,
    next_draw: Option<Instant>,
    exit: Arc<AtomicBool>,
    stop: bool,
}

/// Top-left corner that keeps a surface of `size` inside `screen` (corner first if too large).
pub fn clamp_to_screen(
    (x, y): (i32, i32),
    (width, height): (u32, u32),
    (screen_w, screen_h): (f32, f32),
) -> (i32, i32) {
    let max_x = (screen_w as i32 - width as i32).max(0);
    let max_y = (screen_h as i32 - height as i32).max(0);
    (x.clamp(0, max_x), y.clamp(0, max_y))
}

fn wlr_anchor(anchor: Anchor) -> WlrAnchor {
    let mut out = WlrAnchor::empty();
    if anchor.top {
        out |= WlrAnchor::TOP;
    }
    if anchor.bottom {
        out |= WlrAnchor::BOTTOM;
    }
    if anchor.left {
        out |= WlrAnchor::LEFT;
    }
    if anchor.right {
        out |= WlrAnchor::RIGHT;
    }
    out
}

fn wlr_layer(placement: LayerPlacement) -> Layer {
    match placement {
        LayerPlacement::Bottom => Layer::Bottom,
        LayerPlacement::Top => Layer::Top,
    }
}

/// Run the surface until it is closed or `exit` is set. Blocks the calling thread.
pub fn run(
    request: Request,
    app: Box<dyn Frame>,
    ctx: egui::Context,
    exit: Arc<AtomicBool>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
    let (globals, event_queue) = registry_queue_init(&conn).map_err(|e| e.to_string())?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let shell = LayerShell::bind(&globals, &qh).map_err(|_| "No layer-shell".to_string())?;
    let placement = layer_for(request.layer);
    let surface = compositor.create_surface(&qh);
    let layer =
        shell.create_layer_surface(&qh, surface, wlr_layer(placement), Some("status-quo"), None);
    layer.set_anchor(wlr_anchor(request.anchor));
    layer.set_size(request.width, request.height);
    layer.set_margin(
        request.margin,
        request.margin,
        request.margin,
        request.margin,
    );
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let display = NonNull::new(conn.backend().display_ptr() as *mut _).ok_or("No display")?;
    let window = NonNull::new(layer.wl_surface().id().as_ptr() as *mut _).ok_or("No surface")?;
    let wgpu_surface = unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display)),
                raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
            })
            .map_err(|e| format!("GPU surface: {e}"))?
    };
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&wgpu_surface),
        ..Default::default()
    }))
    .ok_or("No GPU adapter")?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("status-quo"),
            ..Default::default()
        },
        None,
    ))
    .map_err(|e| format!("GPU device: {e}"))?;
    let capabilities = wgpu_surface.get_capabilities(&adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .unwrap_or(capabilities.formats[0]);
    let alpha_mode = if capabilities
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
    {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        wgpu::CompositeAlphaMode::Auto
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: request.width,
        height: request.height,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    };
    wgpu_surface.configure(&device, &config);
    let renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);

    ctx.set_embed_viewports(true);
    let mut event_loop: EventLoop<State> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(handle.clone())
        .map_err(|e| e.to_string())?;
    // egui asks for a later frame through this channel; the timer wakes the loop.
    let (wake_send, wake_receive) = calloop::channel::channel::<Duration>();
    ctx.set_request_repaint_callback(move |info| {
        let _ = wake_send.send(info.delay);
    });
    handle
        .insert_source(wake_receive, |event, _, state| {
            if let calloop::channel::Event::Msg(delay) = event {
                state.want_frame(delay);
            }
        })
        .map_err(|e| e.to_string())?;
    handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(200)),
            |_, _, state| {
                if state.exit.load(Ordering::Acquire) {
                    state.stop = true;
                }
                calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(200))
            },
        )
        .map_err(|e| e.to_string())?;

    let mut state = State {
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        outputs: OutputState::new(&globals, &qh),
        layer,
        pointer: None,
        gpu: Some(Gpu {
            device,
            queue,
            surface: wgpu_surface,
            config,
            renderer,
        }),
        adapter,
        ctx,
        app,
        placement,
        size: (request.width, request.height),
        surface: Surface::Legacy,
        anchor: request.anchor,
        margin: request.margin,
        legacy_size: (request.width, request.height),
        scale: 1,
        configured: false,
        events: Vec::new(),
        pointer_pos: egui::Pos2::ZERO,
        next_draw: None,
        exit,
        stop: false,
    };
    while !state.stop {
        let timeout = state
            .next_draw
            .map(|at| at.saturating_duration_since(Instant::now()));
        event_loop
            .dispatch(timeout, &mut state)
            .map_err(|e| format!("Wayland loop: {e}"))?;
        if state.configured
            && (!state.events.is_empty() || state.next_draw.is_some_and(|at| at <= Instant::now()))
        {
            state.draw();
        }
    }
    // The GPU surface must go before the layer surface it draws on.
    drop(state.gpu.take());
    Ok(())
}

impl State {
    fn want_frame(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        self.next_draw = Some(self.next_draw.map_or(at, |old| old.min(at)));
    }

    fn draw(&mut self) {
        self.next_draw = None;
        let wanted = layer_for(self.app.layer());
        if wanted != self.placement {
            self.placement = wanted;
            self.layer.set_layer(wlr_layer(wanted));
        }
        let scale = self.scale.max(1) as f32;
        let size = egui::vec2(self.size.0 as f32, self.size.1 as f32);
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(scale);
        let mut keep_open = true;
        let output = self.ctx.run(input, |ctx| {
            keep_open = self.app.update(ctx);
        });
        if !keep_open {
            self.stop = true;
        }
        for viewport in output.viewport_output.values() {
            if viewport
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
            {
                self.stop = true;
            }
        }
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            if viewport.repaint_delay < Duration::from_secs(3600) {
                self.want_frame(viewport.repaint_delay);
            }
        }
        let wanted_surface = self.app.surface();
        if wanted_surface != self.surface {
            self.apply_surface(wanted_surface);
        }
        let Some(gpu) = self.gpu.as_mut() else { return };
        let jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            gpu.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.config.width, gpu.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        gpu.renderer
            .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        let frame = match gpu.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(_) => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                self.want_frame(Duration::from_millis(100));
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("status-quo"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            gpu.renderer.render(&mut pass, &jobs, &screen);
        }
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        for id in &output.textures_delta.free {
            gpu.renderer.free_texture(id);
        }
    }

    /// Logical size of the first output, for placing and covering the desktop.
    fn screen(&self) -> (f32, f32) {
        self.outputs
            .outputs()
            .find_map(|o| self.outputs.info(&o))
            .and_then(|i| i.logical_size)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or((1920.0, 1080.0))
    }

    /// Where the docked surface's top-left corner is on the desktop right now.
    fn docked_origin(&self) -> (f32, f32) {
        match self.surface {
            Surface::Free {
                x,
                y,
                width,
                height,
            } => {
                let (x, y) = clamp_to_screen((x, y), (width, height), self.screen());
                (x as f32, y as f32)
            }
            _ => {
                let (sw, sh) = self.screen();
                let (w, h) = (self.size.0 as f32, self.size.1 as f32);
                let m = self.margin as f32;
                let x = if self.anchor.left {
                    m
                } else if self.anchor.right {
                    sw - m - w
                } else {
                    (sw - w) / 2.0
                };
                let y = if self.anchor.top {
                    m
                } else if self.anchor.bottom {
                    sh - m - h
                } else {
                    (sh - h) / 2.0
                };
                (x.max(0.0), y.max(0.0))
            }
        }
    }

    fn apply_surface(&mut self, wanted: Surface) {
        if wanted == Surface::Editing && self.surface != Surface::Editing {
            let origin = self.docked_origin();
            self.app.docked_origin(origin);
        }
        match wanted {
            Surface::Legacy => {
                self.layer.set_anchor(wlr_anchor(self.anchor));
                let m = self.margin;
                self.layer.set_margin(m, m, m, m);
                self.layer.set_size(self.legacy_size.0, self.legacy_size.1);
            }
            Surface::Free {
                x,
                y,
                width,
                height,
            } => {
                // Keep the docked tray on the screen: a tray wider or lower than where it was
                // left slides back, so its header buttons stay reachable.
                let (screen_w, screen_h) = self.screen();
                let (x, y) = clamp_to_screen((x, y), (width, height), (screen_w, screen_h));
                self.layer.set_anchor(WlrAnchor::TOP | WlrAnchor::LEFT);
                self.layer.set_margin(y, 0, 0, x);
                self.layer.set_size(width.max(1), height.max(1));
            }
            Surface::Editing => {
                self.layer.set_anchor(
                    WlrAnchor::TOP | WlrAnchor::BOTTOM | WlrAnchor::LEFT | WlrAnchor::RIGHT,
                );
                self.layer.set_margin(0, 0, 0, 0);
                self.layer.set_size(0, 0);
            }
        }
        self.layer.commit();
        self.surface = wanted;
        self.want_frame(Duration::ZERO);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        let scale = self.scale.max(1) as u32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.config.width = self.size.0 * scale;
            gpu.config.height = self.size.1 * scale;
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
        self.layer.wl_surface().set_buffer_scale(self.scale.max(1));
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        self.scale = factor.max(1);
        self.resize(self.size.0, self.size.1);
        self.want_frame(Duration::ZERO);
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.stop = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        let (w, h) = (
            if w == 0 { self.size.0 } else { w },
            if h == 0 { self.size.1 } else { h },
        );
        self.resize(w, h);
        self.configured = true;
        let _ = &self.adapter;
        self.want_frame(Duration::ZERO);
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Map a Wayland pointer button code to egui. Other buttons are ignored.
pub fn button(code: u32) -> Option<egui::PointerButton> {
    match code {
        BTN_LEFT => Some(egui::PointerButton::Primary),
        BTN_RIGHT => Some(egui::PointerButton::Secondary),
        BTN_MIDDLE => Some(egui::PointerButton::Middle),
        _ => None,
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let position = egui::pos2(event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_pos = position;
                    self.events.push(egui::Event::PointerMoved(position));
                }
                PointerEventKind::Leave { .. } => self.events.push(egui::Event::PointerGone),
                PointerEventKind::Press { button: code, .. }
                | PointerEventKind::Release { button: code, .. } => {
                    if let Some(button) = button(code) {
                        let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                        self.events.push(egui::Event::PointerButton {
                            pos: position,
                            button,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    self.events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(-horizontal.absolute as f32, -vertical.absolute as f32),
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
    }
}

delegate_compositor!(State);
delegate_output!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_layer!(State);
delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-27: mouse buttons map to egui; unknown buttons are ignored.
    #[test]
    fn pointer_buttons_map_to_egui_buttons() {
        assert_eq!(button(0x110), Some(egui::PointerButton::Primary));
        assert_eq!(button(0x111), Some(egui::PointerButton::Secondary));
        assert_eq!(button(0x112), Some(egui::PointerButton::Middle));
        assert_eq!(button(0x113), None);
    }

    // REQ SQ-27: every anchor flag reaches the compositor request.
    // REQ SQ-15 SQ-29: a docked tray never slides off the screen, so its buttons stay reachable.
    #[test]
    fn docked_tray_is_kept_on_the_screen() {
        let screen = (1920.0, 1080.0);
        assert_eq!(clamp_to_screen((1464, 140), (540, 986), screen), (1380, 94));
        assert_eq!(clamp_to_screen((100, 50), (400, 300), screen), (100, 50));
        assert_eq!(clamp_to_screen((-30, -5), (400, 300), screen), (0, 0));
        assert_eq!(clamp_to_screen((500, 500), (3000, 2000), screen), (0, 0));
    }

    #[test]
    fn anchor_flags_translate_to_layer_shell_edges() {
        let corner = wlr_anchor(Anchor {
            top: true,
            right: true,
            ..Default::default()
        });
        assert!(corner.contains(WlrAnchor::TOP | WlrAnchor::RIGHT));
        assert!(!corner.contains(WlrAnchor::LEFT));
    }
}
