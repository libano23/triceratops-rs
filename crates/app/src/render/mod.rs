//! wgpu renderer: shaded meshes, lines and points with 4x MSAA, plus egui on top.

pub mod scene;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4};
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::camera::{Camera, ViewRect};

pub const MSAA_SAMPLES: u32 = 4;
pub const MAX_VIEWPORTS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    /// x = gloss (0..1), y = unused
    pub extra: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointInstance {
    pub pos: [f32; 3],
    pub size: f32,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    viewport: [f32; 4],
    /// xyz = light direction, w = 1 when a clipping plane is active
    light: [f32; 4],
    background: [f32; 4],
    /// clipping plane: xyz = normal, w = offset; fragments with dot(n, p) + w > 0 are discarded
    clip: [f32; 4],
    /// gradient background bottom colour; w = 1 when a gradient is active
    background2: [f32; 4],
    /// x, y = viewport origin in pixels, z = number of scene lights, w = unused
    origin: [f32; 4],
    /// up to 8 lights, 3 vec4 each: (pos/dir xyz, type), (direction xyz, spot angle), (colour rgb, intensity)
    lights: [[f32; 4]; 24],
}

/// A textured mesh group inside a batch (one texture file).
#[derive(Clone, Default)]
pub struct TexturedMesh {
    pub key: String,
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

/// CPU-side geometry for one frame of one viewport.
#[derive(Default)]
pub struct Batch {
    pub lines: Vec<LineVertex>,
    /// Lines drawn without depth test (X-Ray mode, tracking lines).
    pub xray_lines: Vec<LineVertex>,
    pub mesh_vertices: Vec<MeshVertex>,
    pub mesh_indices: Vec<u32>,
    pub points: Vec<PointInstance>,
    pub textured: Vec<TexturedMesh>,
}

impl Batch {
    /// Shaded mesh with a gloss value and per-vertex texture coordinates.
    pub fn mesh_ex(&mut self, m: &tri_geom::Mesh, color: [f32; 4], gloss: f32, uvs: Option<&[[f32; 2]]>, texture: Option<&str>) {
        let verts: Vec<MeshVertex> = m
            .vertices
            .iter()
            .zip(&m.normals)
            .enumerate()
            .map(|(i, (p, n))| MeshVertex { pos: p.as_vec3().to_array(), normal: n.as_vec3().to_array(), color, uv: uvs.and_then(|u| u.get(i).copied()).unwrap_or([0.0, 0.0]), extra: [gloss, 0.0] })
            .collect();
        match texture {
            Some(key) => {
                let group = match self.textured.iter_mut().find(|t| t.key == key) {
                    Some(g) => g,
                    None => {
                        self.textured.push(TexturedMesh { key: key.to_string(), ..Default::default() });
                        self.textured.last_mut().unwrap()
                    }
                };
                let base = group.vertices.len() as u32;
                group.vertices.extend(verts);
                group.indices.extend(m.indices.iter().map(|i| i + base));
            }
            None => {
                let base = self.mesh_vertices.len() as u32;
                self.mesh_vertices.extend(verts);
                self.mesh_indices.extend(m.indices.iter().map(|i| i + base));
            }
        }
    }

    pub fn line(&mut self, a: DVec3, b: DVec3, color: [f32; 4]) {
        self.lines.push(LineVertex { pos: a.as_vec3().to_array(), color });
        self.lines.push(LineVertex { pos: b.as_vec3().to_array(), color });
    }

    pub fn segments(&mut self, pts: &[DVec3], color: [f32; 4]) {
        for seg in pts.chunks(2) {
            if seg.len() == 2 {
                self.line(seg[0], seg[1], color);
            }
        }
    }

    pub fn polyline(&mut self, pts: &[DVec3], color: [f32; 4]) {
        for w in pts.windows(2) {
            self.line(w[0], w[1], color);
        }
    }

    pub fn mesh(&mut self, m: &tri_geom::Mesh, color: [f32; 4]) {
        let base = self.mesh_vertices.len() as u32;
        for (p, n) in m.vertices.iter().zip(&m.normals) {
            self.mesh_vertices.push(MeshVertex { pos: p.as_vec3().to_array(), normal: n.as_vec3().to_array(), color, uv: [0.0, 0.0], extra: [0.3, 0.0] });
        }
        self.mesh_indices.extend(m.indices.iter().map(|i| i + base));
    }

    pub fn point(&mut self, p: DVec3, size: f32, color: [f32; 4]) {
        self.points.push(PointInstance { pos: p.as_vec3().to_array(), size, color });
    }

    pub fn xray_line(&mut self, a: DVec3, b: DVec3, color: [f32; 4]) {
        self.xray_lines.push(LineVertex { pos: a.as_vec3().to_array(), color });
        self.xray_lines.push(LineVertex { pos: b.as_vec3().to_array(), color });
    }

}

struct GrowBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let capacity = 64 * 1024;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, capacity, usage, label }
    }

    fn write<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64).next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity,
                usage: self.usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }
}

struct TexSet {
    key: String,
    v: GrowBuffer,
    i: GrowBuffer,
    count: u32,
}

struct DrawSet {
    lines: GrowBuffer,
    xray: GrowBuffer,
    mesh_v: GrowBuffer,
    mesh_i: GrowBuffer,
    points: GrowBuffer,
    line_count: u32,
    xray_count: u32,
    index_count: u32,
    point_count: u32,
    tex: Vec<TexSet>,
}

impl DrawSet {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            lines: GrowBuffer::new(device, "lines", wgpu::BufferUsages::VERTEX),
            xray: GrowBuffer::new(device, "xray lines", wgpu::BufferUsages::VERTEX),
            mesh_v: GrowBuffer::new(device, "mesh vertices", wgpu::BufferUsages::VERTEX),
            mesh_i: GrowBuffer::new(device, "mesh indices", wgpu::BufferUsages::INDEX),
            points: GrowBuffer::new(device, "points", wgpu::BufferUsages::VERTEX),
            line_count: 0,
            xray_count: 0,
            index_count: 0,
            point_count: 0,
            tex: Vec::new(),
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, batch: &Batch) {
        self.lines.write(device, queue, &batch.lines);
        self.xray.write(device, queue, &batch.xray_lines);
        self.xray_count = batch.xray_lines.len() as u32;
        self.mesh_v.write(device, queue, &batch.mesh_vertices);
        self.mesh_i.write(device, queue, &batch.mesh_indices);
        self.points.write(device, queue, &batch.points);
        self.line_count = batch.lines.len() as u32;
        self.index_count = batch.mesh_indices.len() as u32;
        self.point_count = batch.points.len() as u32;
        self.tex.truncate(batch.textured.len());
        for (k, t) in batch.textured.iter().enumerate() {
            if self.tex.len() <= k {
                self.tex.push(TexSet { key: String::new(), v: GrowBuffer::new(device, "tex vertices", wgpu::BufferUsages::VERTEX), i: GrowBuffer::new(device, "tex indices", wgpu::BufferUsages::INDEX), count: 0 });
            }
            let set = &mut self.tex[k];
            set.key = t.key.clone();
            set.v.write(device, queue, &t.vertices);
            set.i.write(device, queue, &t.indices);
            set.count = t.indices.len() as u32;
        }
    }
}

/// GPU buffers for one viewport: static scene plus per-frame overlay.
struct ViewportBuffers {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    scene: DrawSet,
    overlay: DrawSet,
}

/// Why a frame was not drawn.
#[derive(Debug)]
pub enum FrameError {
    /// The window is hidden or the swapchain timed out: try again next frame.
    Skip,
    /// The surface is lost or no longer matches the window: configure it again.
    Reconfigure,
    /// wgpu reported a validation error while acquiring the frame.
    Validation,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    bg_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    line_xray_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    mesh_tex_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Loaded textures by file path (None = failed to load).
    textures: std::collections::HashMap<String, Option<wgpu::BindGroup>>,
    viewports: Vec<ViewportBuffers>,
    pub egui_renderer: egui_wgpu::Renderer,
    pub background: [f64; 3],
    /// Name of the GPU adapter and backend in use ("... (Gl backend, ...)").
    pub adapter_info: String,
    /// Pending capture: pixel rectangle (x, y, w, h) and whether the UI should be included.
    pub capture: Option<((u32, u32, u32, u32), bool)>,
    /// Result of the last capture: width, height, RGBA8 pixels.
    pub captured: Option<(u32, u32, Vec<u8>)>,
    /// Whether the swapchain supports COPY_SRC (Vulkan yes, OpenGL no): otherwise captures go
    /// through an offscreen texture that is blitted to the window.
    surface_copy: bool,
    blit_pipeline: wgpu::RenderPipeline,
    offscreen: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
}

/// Pending-capture bookkeeping for one frame.
struct CaptureBuffer {
    buffer: wgpu::Buffer,
    w: u32,
    h: u32,
    padded: u32,
}

const BLIT_SHADER: &str = r#"
struct BlitOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;
@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> BlitOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: BlitOut;
    out.pos = vec4<f32>(corners[vi], 0.0, 1.0);
    out.uv = vec2<f32>(corners[vi].x * 0.5 + 0.5, 0.5 - corners[vi].y * 0.5);
    return out;
}
@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(t_src, s_src, in.uv);
}
"#;

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    viewport: vec4<f32>,
    light: vec4<f32>,
    background: vec4<f32>,
    clip: vec4<f32>,
    background2: vec4<f32>,
    origin: vec4<f32>,
    lights: array<vec4<f32>, 24>,
};

@group(1) @binding(0) var t_color: texture_2d<f32>;
@group(1) @binding(1) var s_color: sampler;

fn clipped(world: vec3<f32>) -> bool {
    return u.light.w > 0.5 && dot(u.clip.xyz, world) + u.clip.w > 0.0;
}

struct BgOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_bg(@builtin(vertex_index) vi: u32) -> BgOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: BgOut;
    out.pos = vec4<f32>(corners[vi], 1.0, 1.0);
    return out;
}

@fragment
fn fs_bg(in: BgOut) -> @location(0) vec4<f32> {
    if (u.background2.w > 0.5) {
        let t = clamp((in.pos.y - u.origin.y) / max(u.viewport.y, 1.0), 0.0, 1.0);
        return vec4<f32>(mix(u.background.rgb, u.background2.rgb, t), 1.0);
    }
    return u.background;
}
@group(0) @binding(0) var<uniform> u: Uniforms;

struct LineIn { @location(0) pos: vec3<f32>, @location(1) color: vec4<f32> };
struct LineOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32>, @location(1) world: vec3<f32> };

@vertex
fn vs_line(in: LineIn) -> LineOut {
    var out: LineOut;
    out.pos = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    out.world = in.pos;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) { discard; }
    return in.color;
}

struct MeshIn { @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) uv: vec2<f32>, @location(4) extra: vec2<f32> };
struct MeshOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32>, @location(1) normal: vec3<f32>, @location(2) world: vec3<f32>, @location(3) uv: vec2<f32>, @location(4) extra: vec2<f32> };

@vertex
fn vs_mesh(in: MeshIn) -> MeshOut {
    var out: MeshOut;
    var p = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.pos = p;
    out.color = in.color;
    out.normal = in.normal;
    out.world = in.pos;
    out.uv = in.uv;
    out.extra = in.extra;
    return out;
}

/// Lighting: headlight when no scene lights, otherwise ambient plus the scene lights.
fn shade(n0: vec3<f32>, world: vec3<f32>, base: vec3<f32>, gloss: f32) -> vec3<f32> {
    let v = normalize(u.eye.xyz - world);
    var n = n0;
    if (dot(n, v) < 0.0) { n = -n; }
    let exponent = mix(8.0, 96.0, clamp(gloss, 0.0, 1.0));
    let spec_w = 0.05 + 0.5 * clamp(gloss, 0.0, 1.0);
    let count = u32(u.origin.z + 0.5);
    if (count == 0u) {
        let l = normalize(u.light.xyz);
        let diffuse = abs(dot(n, l));
        let head = abs(dot(n, v));
        let h = normalize(l + v);
        let spec = pow(abs(dot(n, h)), exponent) * spec_w;
        let s = 0.30 + 0.45 * diffuse + 0.25 * head;
        return base * s + vec3<f32>(spec);
    }
    var c = base * 0.22;
    for (var i = 0u; i < count; i = i + 1u) {
        let a = u.lights[i * 3u];
        let b = u.lights[i * 3u + 1u];
        let col = u.lights[i * 3u + 2u];
        var l: vec3<f32>;
        var att = 1.0;
        if (a.w < 0.5) {
            l = normalize(-b.xyz);
        } else {
            let d = a.xyz - world;
            l = normalize(d);
            if (a.w > 1.5) {
                // spotlight cone
                let cosang = dot(-l, normalize(b.xyz));
                let edge = cos(b.w);
                att = smoothstep(edge, edge + 0.05, cosang);
            }
        }
        let diffuse = max(dot(n, l), 0.0);
        let h = normalize(l + v);
        let spec = pow(max(dot(n, h), 0.0), exponent) * spec_w;
        c = c + (base * diffuse + vec3<f32>(spec)) * col.rgb * col.w * att;
    }
    return c;
}

fn colormap(t: f32) -> vec3<f32> {
    // blue -> cyan -> green -> yellow -> red
    let x = clamp(t, 0.0, 1.0) * 4.0;
    if (x < 1.0) { return vec3<f32>(0.0, x, 1.0); }
    if (x < 2.0) { return vec3<f32>(0.0, 1.0, 2.0 - x); }
    if (x < 3.0) { return vec3<f32>(x - 2.0, 1.0, 0.0); }
    return vec3<f32>(1.0, 4.0 - x, 0.0);
}

@fragment
fn fs_mesh(in: MeshOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) { discard; }
    let n = normalize(in.normal);
    let l = normalize(u.light.xyz);
    let v = normalize(u.eye.xyz - in.world);
    let mode = u32(u.viewport.w + 0.5);
    if (mode == 0u) {
        return vec4<f32>(shade(n, in.world, in.color.rgb, in.extra.x), in.color.a);
    }
    if (mode == 1u) {
        // zebra: stripes from the reflection vector
        let r = reflect(-v, n);
        let s = fract(r.z * 6.0 + r.x * 2.0);
        let c = select(0.1, 0.95, s < 0.5);
        return vec4<f32>(c, c, c, in.color.a);
    }
    if (mode == 2u) {
        // environment map: sky/ground gradient from the reflection vector
        let r = reflect(-v, n);
        let sky = vec3<f32>(0.55, 0.75, 1.0);
        let ground = vec3<f32>(0.35, 0.3, 0.25);
        let horizon = vec3<f32>(0.95, 0.95, 0.9);
        let t = r.z;
        var c: vec3<f32>;
        if (t > 0.0) { c = mix(horizon, sky, t); } else { c = mix(horizon, ground, -t); }
        return vec4<f32>(c, in.color.a);
    }
    if (mode == 3u) {
        // curvature estimate from the change of the normal per unit length
        let dn = length(dpdx(n)) + length(dpdy(n));
        let dp = length(dpdx(in.world)) + length(dpdy(in.world));
        let k = dn / max(dp, 1e-6);
        return vec4<f32>(colormap(k * 2.0), in.color.a);
    }
    if (mode == 4u) {
        // draft angle: angle between the normal and the world Z axis
        let a = degrees(acos(clamp(n.z, -1.0, 1.0)));
        return vec4<f32>(colormap(a / 180.0), in.color.a);
    }
    if (mode == 5u) {
        // z-buffer: grey by distance from the eye
        let d = length(u.eye.xyz - in.world);
        let g = clamp(1.0 - d / (2.0 * length(u.eye.xyz) + 1.0), 0.0, 1.0);
        return vec4<f32>(g, g, g, in.color.a);
    }
    if (mode == 6u) {
        // thickness analysis: distance to the opposite side stored in uv.x (fraction of the object size)
        return vec4<f32>(colormap(1.0 - clamp(in.uv.x * 2.0, 0.0, 1.0)), in.color.a);
    }
    let diffuse = abs(dot(n, l));
    let head = abs(dot(n, v));
    let h = normalize(l + v);
    let spec = pow(abs(dot(n, h)), 32.0) * 0.15;
    let shade_v = 0.30 + 0.45 * diffuse + 0.25 * head;
    return vec4<f32>(in.color.rgb * shade_v + vec3<f32>(spec), in.color.a);
}

@fragment
fn fs_mesh_tex(in: MeshOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) { discard; }
    let n = normalize(in.normal);
    let tex = textureSample(t_color, s_color, in.uv);
    let base = in.color.rgb * tex.rgb;
    return vec4<f32>(shade(n, in.world, base, in.extra.x), in.color.a * tex.a);
}

struct PointIn { @location(0) pos: vec3<f32>, @location(1) size: f32, @location(2) color: vec4<f32> };
struct PointOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32>, @location(1) world: vec3<f32> };

@vertex
fn vs_point(@builtin(vertex_index) vi: u32, in: PointIn) -> PointOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    var out: PointOut;
    var p = u.view_proj * vec4<f32>(in.pos, 1.0);
    let c = corners[vi];
    let px = in.size * u.viewport.z;
    p.x = p.x + c.x * px / u.viewport.x * 2.0 * p.w;
    p.y = p.y + c.y * px / u.viewport.y * 2.0 * p.w;
    p.z = p.z - 0.0002 * p.w;
    out.pos = p;
    out.color = in.color;
    out.world = in.pos;
    return out;
}

@fragment
fn fs_point(in: PointOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) { discard; }
    return in.color;
}
"#;

impl Renderer {
    pub fn new(window: Arc<Window>, backend: &str) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let backends = match backend.to_ascii_lowercase().as_str() {
            "gl" | "opengl" | "gles" => wgpu::Backends::GL,
            "vulkan" | "vk" => wgpu::Backends::VULKAN,
            _ => wgpu::Backends::VULKAN | wgpu::Backends::GL,
        };
        // GLES (notably on Wayland) presents through the display connection, so pass it on
        let mut instance_desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()));
        instance_desc.backends = backends;
        let instance = wgpu::Instance::new(instance_desc);
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        log::info!("adapter: {:?}", adapter.get_info());
        let info = adapter.get_info();
        let adapter_info = format!("{} ({:?} backend, {:?})", info.name, info.backend, info.device_type);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("triceratops device"),
            required_features: wgpu::Features::empty(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow::anyhow!("surface not supported by adapter"))?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let surface_copy = caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT | if surface_copy { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene layout"),
            bind_group_layouts: &[Some(&bind_layout)], immediate_size: 0,
        });
        let depth_state = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        };
        // surfaces are pushed back by a depth bias (in depth-buffer units, so it works at any
        // zoom) so that curves and isocurves lying on them stay visible
        let depth_state_biased = || wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState { constant: 4, slope_scale: 2.0, clamp: 0.0 },
        };
        let multisample = wgpu::MultisampleState { count: MSAA_SAMPLES, ..Default::default() };
        let target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_bg"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bg"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::LessEqual)),
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let line_xray_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines xray"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let tex_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("textured"), bind_group_layouts: &[Some(&bind_layout), Some(&tex_layout)], immediate_size: 0 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("texture sampler"), address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blit"), source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()) });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("blit"), bind_group_layouts: &[Some(&tex_layout)], immediate_size: 0 });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState { module: &blit_shader, entry_point: Some("vs_blit"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &blit_shader, entry_point: Some("fs_blit"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let mesh_tex_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh textured"),
            layout: Some(&tex_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mesh"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2],
                })],
            },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_mesh_tex"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(depth_state_biased()),
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mesh"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mesh"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state_biased()),
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let point_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("points"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_point"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PointInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_point"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::LessEqual)),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        let (msaa_view, depth_view) = Self::create_targets(&device, &config);
        let mut viewports = Vec::new();
        for _ in 0..MAX_VIEWPORTS {
            viewports.push(Self::make_viewport_buffers(&device, &bind_layout));
        }
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        Ok(Self {
            device,
            queue,
            surface,
            config,
            msaa_view,
            depth_view,
            bg_pipeline,
            line_pipeline,
            line_xray_pipeline,
            mesh_pipeline,
            mesh_tex_pipeline,
            point_pipeline,
            bind_layout,
            tex_layout,
            sampler,
            textures: std::collections::HashMap::new(),
            viewports,
            egui_renderer,
            background: [0.27, 0.27, 0.27],
            adapter_info,
            surface_copy,
            blit_pipeline,
            offscreen: None,
            capture: None,
            captured: None,
        })
    }

    fn make_viewport_buffers(device: &wgpu::Device, bind_layout: &wgpu::BindGroupLayout) -> ViewportBuffers {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport uniforms"),
            contents: bytemuck::bytes_of(&Uniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport bind group"),
            layout: bind_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        ViewportBuffers { uniform, bind_group, scene: DrawSet::new(device), overlay: DrawSet::new(device) }
    }

    /// Grow the per-viewport buffers so `n` viewports can be drawn (details, NewViewport).
    pub fn ensure_viewports(&mut self, n: usize) {
        while self.viewports.len() < n {
            let vb = Self::make_viewport_buffers(&self.device, &self.bind_layout);
            self.viewports.push(vb);
        }
    }

    fn create_targets(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> (wgpu::TextureView, wgpu::TextureView) {
        let size = wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 };
        let msaa = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("msaa color"),
            size,
            mip_level_count: 1,
            sample_count: MSAA_SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size,
            mip_level_count: 1,
            sample_count: MSAA_SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        (msaa.create_view(&Default::default()), depth.create_view(&Default::default()))
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let (m, d) = Self::create_targets(&self.device, &self.config);
        self.msaa_view = m;
        self.depth_view = d;
        let _ = &self.bind_layout;
    }

    /// Upload a viewport's camera and, optionally, its static scene batch.
    pub fn upload(&mut self, index: usize, scene: Option<&Batch>, overlay: &Batch, cam: &Camera, rect: &ViewRect, ppp: f64, background: [f32; 4], clip: Option<[f32; 4]>, sun: Option<glam::DVec3>, analysis: u32, gradient: Option<[f32; 3]>, lights: &[[f32; 4]]) {
        // make sure every texture used by the scene is loaded
        if let Some(b) = scene {
            for t in &b.textured {
                self.ensure_texture(&t.key);
            }
        }
        self.ensure_viewports(index + 1);
        let vb = &mut self.viewports[index];
        let vp = cam.view_proj(rect.aspect());
        let eye = cam.eye();
        let (right, up) = cam.basis();
        let light = sun.unwrap_or_else(|| (cam.eye_dir() + right * 0.5 + up * 0.8).normalize());
        let u = Uniforms {
            view_proj: Mat4::from_cols_array_2d(&vp.as_mat4().to_cols_array_2d()).to_cols_array_2d(),
            eye: [eye.x as f32, eye.y as f32, eye.z as f32, 1.0],
            viewport: [(rect.w * ppp) as f32, (rect.h * ppp) as f32, ppp as f32, analysis as f32],
            light: [light.x as f32, light.y as f32, light.z as f32, if clip.is_some() { 1.0 } else { 0.0 }],
            background,
            clip: clip.unwrap_or([0.0; 4]),
            background2: gradient.map(|g| [g[0], g[1], g[2], 1.0]).unwrap_or([0.0; 4]),
            origin: [(rect.x * ppp) as f32, (rect.y * ppp) as f32, (lights.len() / 3).min(8) as f32, 0.0],
            lights: {
                let mut arr = [[0.0f32; 4]; 24];
                for (i, l) in lights.iter().take(24).enumerate() {
                    arr[i] = *l;
                }
                arr
            },
        };
        self.queue.write_buffer(&vb.uniform, 0, bytemuck::bytes_of(&u));
        if let Some(b) = scene {
            vb.scene.write(&self.device, &self.queue, b);
        }
        vb.overlay.write(&self.device, &self.queue, overlay);
    }

    /// Render all viewports and the egui overlay.
    pub fn render(
        &mut self,
        viewports: &[(usize, ViewRect)],
        ppp: f64,
        egui_ctx: &egui::Context,
        egui_output: egui::FullOutput,
    ) -> Result<(), FrameError> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::Skip),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::Reconfigure),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::Validation),
        };
        let surface_view = frame.texture.create_view(&Default::default());
        // OpenGL swapchains cannot be copied: capture frames render into an offscreen texture
        let use_offscreen = self.capture.is_some() && !self.surface_copy;
        if use_offscreen {
            self.ensure_offscreen();
        }
        let view = if use_offscreen { self.offscreen.as_ref().unwrap().1.clone() } else { surface_view.clone() };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        let paint_jobs = egui_ctx.tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        self.egui_renderer.update_buffers(&self.device, &self.queue, &mut encoder, &paint_jobs, &screen);

        {
            let bg = self.background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: bg[0], g: bg[1], b: bg[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let (fw, fh) = (self.config.width as f64, self.config.height as f64);
            for (idx, rect) in viewports {
                let x = (rect.x * ppp).round().clamp(0.0, fw);
                let y = (rect.y * ppp).round().clamp(0.0, fh);
                let w = (rect.w * ppp).round().clamp(1.0, fw - x);
                let h = (rect.h * ppp).round().clamp(1.0, fh - y);
                if w < 1.0 || h < 1.0 {
                    continue;
                }
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
                let vb = &self.viewports[*idx];
                pass.set_bind_group(0, &vb.bind_group, &[]);
                pass.set_pipeline(&self.bg_pipeline);
                pass.draw(0..3, 0..1);
                for set in [&vb.scene, &vb.overlay] {
                    if set.index_count > 0 {
                        pass.set_pipeline(&self.mesh_pipeline);
                        pass.set_vertex_buffer(0, set.mesh_v.buffer.slice(..));
                        pass.set_index_buffer(set.mesh_i.buffer.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..set.index_count, 0, 0..1);
                    }
                    for ts in &set.tex {
                        if ts.count == 0 {
                            continue;
                        }
                        if let Some(Some(bg)) = self.textures.get(&ts.key) {
                            pass.set_pipeline(&self.mesh_tex_pipeline);
                            pass.set_bind_group(1, bg, &[]);
                            pass.set_vertex_buffer(0, ts.v.buffer.slice(..));
                            pass.set_index_buffer(ts.i.buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..ts.count, 0, 0..1);
                        } else {
                            pass.set_pipeline(&self.mesh_pipeline);
                            pass.set_vertex_buffer(0, ts.v.buffer.slice(..));
                            pass.set_index_buffer(ts.i.buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..ts.count, 0, 0..1);
                        }
                    }
                    if set.line_count > 0 {
                        pass.set_pipeline(&self.line_pipeline);
                        pass.set_vertex_buffer(0, set.lines.buffer.slice(..));
                        pass.draw(0..set.line_count, 0..1);
                    }
                    if set.xray_count > 0 {
                        pass.set_pipeline(&self.line_xray_pipeline);
                        pass.set_vertex_buffer(0, set.xray.buffer.slice(..));
                        pass.draw(0..set.xray_count, 0..1);
                    }
                    if set.point_count > 0 {
                        pass.set_pipeline(&self.point_pipeline);
                        pass.set_vertex_buffer(0, set.points.buffer.slice(..));
                        pass.draw(0..6, 0..set.point_count);
                    }
                }
            }
            // the OpenGL backend resolves the MSAA target with the scissor still active:
            // restore the full frame so every viewport is resolved
            pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            pass.set_viewport(0.0, 0.0, self.config.width as f32, self.config.height as f32, 0.0, 1.0);
        }
        let mut capture_buf: Option<CaptureBuffer> = None;
        let capture_tex = if use_offscreen { &self.offscreen.as_ref().unwrap().0 } else { &frame.texture };
        if let Some((rect, with_ui)) = self.capture {
            if !with_ui {
                capture_buf = self.copy_region(&mut encoder, capture_tex, rect);
            }
        }
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        if let Some((rect, true)) = self.capture {
            capture_buf = self.copy_region(&mut encoder, capture_tex, rect);
        }
        if use_offscreen {
            let bg = &self.offscreen.as_ref().unwrap().2;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &surface_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        if let Some(cb) = capture_buf {
            self.read_capture(cb);
        }
        self.capture = None;
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        Ok(())
    }

    /// Offscreen colour target of the window size used for captures on backends whose
    /// swapchain cannot be read back (OpenGL).
    fn ensure_offscreen(&mut self) {
        let (w, h) = (self.config.width.max(1), self.config.height.max(1));
        if let Some((t, _, _)) = &self.offscreen {
            if t.width() == w && t.height() == h {
                return;
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen frame"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offscreen bind group"),
            layout: &self.tex_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
        });
        self.offscreen = Some((texture, view, bind_group));
    }

    /// Load an image file into a texture bind group (cached by path).
    fn ensure_texture(&mut self, key: &str) {
        if self.textures.contains_key(key) {
            return;
        }
        let loaded = image::open(key).ok().map(|img| {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("material texture"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            let view = texture.create_view(&Default::default());
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("texture bind group"),
                layout: &self.tex_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
            })
        });
        if loaded.is_none() {
            log::warn!("texture {key} could not be loaded");
        }
        self.textures.insert(key.to_string(), loaded);
    }

    /// Forget loaded textures so they are re-read from disk.
    pub fn refresh_textures(&mut self) {
        self.textures.clear();
    }

    fn copy_region(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, rect: (u32, u32, u32, u32)) -> Option<CaptureBuffer> {
        let (x, y, w, h) = rect;
        let w = w.min(self.config.width.saturating_sub(x)).max(1);
        let h = h.min(self.config.height.saturating_sub(y)).max(1);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture"), size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        Some(CaptureBuffer { buffer, w, h, padded })
    }

    fn read_capture(&mut self, cb: CaptureBuffer) {
        let slice = cb.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().map(|r| r.is_ok()).unwrap_or(false) {
            let Ok(data) = slice.get_mapped_range() else {
                cb.buffer.unmap();
                return;
            };
            let mut out = Vec::with_capacity((cb.w * cb.h * 4) as usize);
            let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
            for row in 0..cb.h {
                let start = (row * cb.padded) as usize;
                let line = &data[start..start + (cb.w * 4) as usize];
                for px in line.chunks(4) {
                    if bgra {
                        out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    } else {
                        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                }
            }
            drop(data);
            cb.buffer.unmap();
            self.captured = Some((cb.w, cb.h, out));
        }
    }
}
