mod app;
mod file_dialog;
mod camera;
mod commands;
mod input;
mod locust;
mod render;
mod snap;
mod theme;
mod ui;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use app::App;
use render::Renderer;

struct Runner {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    app: App,
}

impl Runner {
    fn new() -> Self {
        let mut app = App::new();
        // `triceratops model.3dm` opens a model; `triceratops --script file.txt` runs a command file at startup.
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--script" && i + 1 < args.len() {
                app.run_command_file(&args[i + 1]);
                i += 2;
            } else {
                app.open_path(args[i].clone().into());
                i += 1;
            }
        }
        Self { window: None, renderer: None, egui_ctx: egui::Context::default(), egui_state: None, app }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(window), Some(renderer), Some(state)) = (&self.window, &mut self.renderer, &mut self.egui_state) else {
            return;
        };
        let raw_input = state.take_egui_input(window);
        let full_output = self.egui_ctx.run(raw_input, |ctx| ui::draw(&mut self.app, ctx));
        state.handle_platform_output(window, full_output.platform_output.clone());
        self.app.process_requests();
        if self.app.quit {
            event_loop.exit();
            return;
        }
        window.set_title(&format!("{} - Triceratops", self.app.doc.title()));
        for op in std::mem::take(&mut self.app.window_ops) {
            match op.as_str() {
                "minimize" => window.set_minimized(true),
                "maximize" => window.set_maximized(!window.is_maximized()),
                "fullscreen" => {
                    if window.fullscreen().is_some() {
                        window.set_fullscreen(None);
                    } else {
                        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
                    }
                }
                _ => {}
            }
        }

        let ppp = full_output.pixels_per_point as f64;
        // animation playback: one frame per rendered frame
        if self.app.animation.playing && !self.app.animation.frames.is_empty() {
            let idx = self.app.animation.index;
            self.app.apply_animation_frame(idx);
            if let Some(dir) = self.app.animation.record_dir.clone() {
                let path = std::path::Path::new(&dir).join(format!("frame{idx:04}.png")).to_string_lossy().to_string();
                self.app.capture_requests.push(app::CaptureRequest { path, clipboard: false, whole_window: false, view: None });
            }
            if idx + 1 >= self.app.animation.frames.len() {
                self.app.animation.playing = false;
                self.app.animation.record_dir = None;
            } else {
                self.app.animation.index = idx + 1;
            }
            self.egui_ctx.request_repaint();
        }
        // pending capture: restore a named view first, then ask the renderer to copy the pixels
        let capture = if self.app.capture_requests.is_empty() { None } else { Some(self.app.capture_requests.remove(0)) };
        if let Some(cr) = &capture {
            if let Some(name) = &cr.view {
                if let Some((_, cam)) = self.app.named_views.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).cloned() {
                    let active = self.app.active;
                    self.app.viewports[active].camera = cam;
                }
            }
            let active = self.app.active;
            let rect = self.app.viewports[active].rect;
            let px = |v: f64| (v * ppp).round().max(0.0) as u32;
            renderer.capture = Some(if cr.whole_window { ((0, 0, renderer.config.width, renderer.config.height), true) } else { ((px(rect.x), px(rect.y), px(rect.w), px(rect.h)), false) });
            self.egui_ctx.request_repaint();
        }
        if std::mem::take(&mut self.app.refresh_textures) {
            renderer.refresh_textures();
        }
        renderer.background = self.app.theme.clear_color();
        let changed = self.app.build_batches();
        let visible = self.app.visible_viewports();
        let mut rects = Vec::new();
        for vp in visible {
            let bg = theme::lin(self.app.background_color(vp));
            let scene = if changed[vp] { Some(self.app.scene_batch(vp)) } else { None };
            let clip = self.app.clip_for(vp);
            let lights = self.app.light_uniforms(vp);
            let gradient = self.app.gradient_for(vp);
            let v = &self.app.viewports[vp];
            renderer.upload(vp, scene, &self.app.overlay_batches[vp], &v.camera, &v.rect, ppp, bg, clip, self.app.sun, v.display.analysis(), gradient, &lights);
            rects.push((vp, v.rect));
        }
        let repaint = full_output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero());
        match renderer.render(&rects, ppp, &self.egui_ctx, full_output) {
            Ok(()) => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let s = window.inner_size();
                renderer.resize(s.width, s.height);
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("out of GPU memory");
                event_loop.exit();
            }
            Err(e) => log::warn!("render error: {e:?}"),
        }
        if let (Some(cr), Some((w, h, data))) = (capture, renderer.captured.take()) {
            self.app.finish_capture(&cr, w, h, data);
        }
        if repaint {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Untitled - Triceratops").with_inner_size(winit::dpi::LogicalSize::new(1680.0, 1020.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let backend = std::env::var("TRICERATOPS_BACKEND").unwrap_or_else(|_| self.app.backend.clone());
        let mut renderer = Renderer::new(window.clone(), &backend).expect("init renderer");
        renderer.background = self.app.theme.clear_color();
        self.app.log.push(format!("Graphics: {}", renderer.adapter_info));
        theme::apply_theme(&self.egui_ctx, &self.app.theme);
        let state = egui_winit::State::new(self.egui_ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, None);
        self.egui_state = Some(state);
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else { return };
        if let Some(state) = &mut self.egui_state {
            let resp = state.on_window_event(&window, &event);
            if resp.repaint {
                window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                self.app.save_options();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => window.request_redraw(),
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,triceratops=info")).init();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut runner = Runner::new();
    event_loop.run_app(&mut runner)?;
    Ok(())
}
