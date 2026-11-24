use glam::{DMat4, DVec2, DVec3, DVec4};
use tri_geom::{BoundingBox, Plane, Ray};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Projection {
    Perspective,
    Parallel,
}

/// Orbit camera. The eye sits at `distance` from `target` along the
/// direction given by `yaw`/`pitch` (radians, Z up).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Camera {
    /// Roll about the viewing direction (TiltView), radians.
    pub roll: f64,
    pub target: DVec3,
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub projection: Projection,
    pub fov_y: f64,
    /// Visible height in world units for parallel projection.
    pub ortho_height: f64,
    /// Explicit up vector for axis-aligned parallel views.
    pub up_override: Option<DVec3>,
    /// Whether orbiting is allowed (false for Top/Front/Right).
    pub rotatable: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self::perspective()
    }
}

impl Camera {
    pub fn perspective() -> Self {
        Self {
            roll: 0.0,
            target: DVec3::ZERO,
            distance: 60.0,
            yaw: (-135.0f64).to_radians(),
            pitch: 30.0f64.to_radians(),
            projection: Projection::Perspective,
            fov_y: 35.0f64.to_radians(),
            ortho_height: 40.0,
            up_override: None,
            rotatable: true,
        }
    }

    pub fn top() -> Self {
        Self {
            pitch: 90.0f64.to_radians(),
            yaw: 0.0,
            projection: Projection::Parallel,
            up_override: Some(DVec3::Y),
            rotatable: false,
            ..Self::perspective()
        }
    }

    pub fn bottom() -> Self {
        Self { pitch: -90.0f64.to_radians(), up_override: Some(DVec3::Y), ..Self::top() }
    }

    pub fn front() -> Self {
        // eye on -Y looking towards +Y
        Self { pitch: 0.0, yaw: (-90.0f64).to_radians(), up_override: Some(DVec3::Z), ..Self::top() }
    }

    pub fn back() -> Self {
        Self { yaw: 90.0f64.to_radians(), ..Self::front() }
    }

    pub fn right() -> Self {
        Self { pitch: 0.0, yaw: 0.0, up_override: Some(DVec3::Z), ..Self::top() }
    }

    pub fn left() -> Self {
        Self { yaw: std::f64::consts::PI, ..Self::right() }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "perspective" | "persp" => Self::perspective(),
            "top" => Self::top(),
            "bottom" => Self::bottom(),
            "front" => Self::front(),
            "back" => Self::back(),
            "right" => Self::right(),
            "left" => Self::left(),
            "isometric" | "iso" | "ne isometric" => {
                let mut c = Self::perspective();
                c.yaw = std::f64::consts::FRAC_PI_4;
                c.pitch = (1.0f64 / 3.0f64.sqrt()).asin();
                c
            }
            _ => return None,
        })
    }

    /// Construction plane naturally associated with this view.
    pub fn default_cplane(&self) -> Plane {
        if self.rotatable {
            return Plane::world_xy();
        }
        let d = self.view_dir();
        if d.z.abs() > 0.9 {
            Plane::world_xy()
        } else if d.y.abs() > 0.9 {
            Plane::world_xz()
        } else {
            Plane::world_yz()
        }
    }

    /// Unit vector from target towards the eye.
    pub fn eye_dir(&self) -> DVec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        DVec3::new(cp * cy, cp * sy, sp)
    }

    /// Viewing direction (eye towards target).
    pub fn view_dir(&self) -> DVec3 {
        -self.eye_dir()
    }

    pub fn eye(&self) -> DVec3 {
        self.target + self.eye_dir() * self.distance
    }

    pub fn up(&self) -> DVec3 {
        if let Some(u) = self.up_override {
            return u;
        }
        let d = self.eye_dir();
        if d.z.abs() > 0.9999 {
            DVec3::Y
        } else {
            DVec3::Z
        }
    }

    pub fn view_matrix(&self) -> DMat4 {
        DMat4::look_at_rh(self.eye(), self.target, self.up())
    }

    pub fn proj_matrix(&self, aspect: f64) -> DMat4 {
        let (near, far) = self.clip_planes();
        match self.projection {
            Projection::Perspective => DMat4::perspective_rh(self.fov_y, aspect, near, far),
            Projection::Parallel => {
                let h = self.ortho_height * 0.5;
                let w = h * aspect;
                DMat4::orthographic_rh(-w, w, -h, h, near, far)
            }
        }
    }

    fn clip_planes(&self) -> (f64, f64) {
        match self.projection {
            Projection::Perspective => {
                let near = (self.distance * 0.002).max(0.01);
                (near, self.distance * 40.0 + 200.0)
            }
            Projection::Parallel => {
                let far = self.distance * 10.0 + self.ortho_height * 10.0 + 100.0;
                (-far, far)
            }
        }
    }

    pub fn view_proj(&self, aspect: f64) -> DMat4 {
        self.proj_matrix(aspect) * self.view_matrix()
    }

    /// Camera right and up vectors in world space.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let f = self.view_dir();
        let r = f.cross(self.up()).normalize_or(DVec3::X);
        let u = r.cross(f).normalize_or(DVec3::Z);
        if self.roll.abs() > 1e-12 {
            let q = glam::DQuat::from_axis_angle(f, self.roll);
            return (q * r, q * u);
        }
        (r, u)
    }

    /// Change the lens while keeping the target's apparent size (DollyZoom).
    pub fn dolly(&mut self, factor: f64) {
        let old = self.fov_y;
        self.fov_y = (old * factor).clamp(1.0f64.to_radians(), 170.0f64.to_radians());
        self.distance = (self.distance * (old * 0.5).tan() / (self.fov_y * 0.5).tan()).max(0.001);
    }

    /// Rotate the view around the eye (RotateCamera): the target moves, the eye stays.
    pub fn rotate_camera(&mut self, dx: f64, dy: f64) {
        let eye = self.eye();
        self.yaw -= dx;
        self.pitch = (self.pitch - dy).clamp(-1.55, 1.55);
        self.target = eye - self.eye_dir() * self.distance;
    }

    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if !self.rotatable {
            return;
        }
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-1.55, 1.55);
    }

    /// Pan by a screen-space delta (points) in a viewport of the given size.
    pub fn pan(&mut self, dx: f64, dy: f64, viewport_h: f64) {
        let world_per_px = self.world_per_pixel(viewport_h);
        let (r, u) = self.basis();
        self.target -= r * (dx * world_per_px);
        self.target += u * (dy * world_per_px);
    }

    /// World units covered by one pixel at the target depth.
    pub fn world_per_pixel(&self, viewport_h: f64) -> f64 {
        match self.projection {
            Projection::Perspective => 2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_h.max(1.0),
            Projection::Parallel => self.ortho_height / viewport_h.max(1.0),
        }
    }

    /// Zoom by a factor (>1 zooms in). If `anchor` is given, keep that world
    /// point fixed under the cursor.
    pub fn zoom(&mut self, factor: f64, anchor: Option<DVec3>) {
        let factor = factor.clamp(0.05, 20.0);
        match self.projection {
            Projection::Perspective => {
                if let Some(a) = anchor {
                    // move target towards the anchor so the point stays put
                    let delta = a - self.target;
                    self.target += delta * (1.0 - 1.0 / factor);
                }
                self.distance = (self.distance / factor).max(0.001);
            }
            Projection::Parallel => {
                if let Some(a) = anchor {
                    let delta = a - self.target;
                    // keep anchor in the view plane: project delta onto camera plane
                    let d = self.view_dir();
                    let delta = delta - d * delta.dot(d);
                    self.target += delta * (1.0 - 1.0 / factor);
                }
                self.ortho_height = (self.ortho_height / factor).max(1e-4);
            }
        }
    }

    pub fn zoom_to(&mut self, bbox: &BoundingBox) {
        if bbox.is_empty() {
            return;
        }
        let center = bbox.center();
        let radius = (bbox.diagonal() * 0.5).max(1.0);
        self.target = center;
        match self.projection {
            Projection::Perspective => {
                self.distance = radius / (self.fov_y * 0.5).sin() * 1.05;
            }
            Projection::Parallel => {
                self.distance = radius * 4.0;
                self.ortho_height = radius * 2.2;
            }
        }
    }

    /// Ray through a point given in normalised device coordinates (-1..1).
    pub fn ray_ndc(&self, ndc: DVec2, aspect: f64) -> Ray {
        let inv = self.view_proj(aspect).inverse();
        let near = inv * DVec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let far = inv * DVec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        Ray::new(near, far - near)
    }

    /// Project a world point to NDC (x, y in -1..1, z depth 0..1). None when behind the eye.
    pub fn project_ndc(&self, p: DVec3, aspect: f64) -> Option<DVec3> {
        let c = self.view_proj(aspect) * p.extend(1.0);
        if c.w <= 1e-9 {
            return None;
        }
        Some(c.truncate() / c.w)
    }
}

/// A viewport rectangle in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl ViewRect {
    pub fn aspect(&self) -> f64 {
        (self.w / self.h.max(1.0)).max(1e-3)
    }

    pub fn screen_to_ndc(&self, x: f64, y: f64) -> DVec2 {
        DVec2::new((x - self.x) / self.w * 2.0 - 1.0, 1.0 - (y - self.y) / self.h * 2.0)
    }

    pub fn ndc_to_screen(&self, ndc: DVec3) -> DVec2 {
        DVec2::new(self.x + (ndc.x + 1.0) * 0.5 * self.w, self.y + (1.0 - ndc.y) * 0.5 * self.h)
    }
}

/// Undo/redo stack for view changes (Home / End keys, UndoView / RedoView).
#[derive(Default, Clone)]
pub struct ViewHistory {
    undo: Vec<Camera>,
    redo: Vec<Camera>,
}

impl ViewHistory {
    pub fn push(&mut self, cam: &Camera) {
        if self.undo.last().is_none_or(|c| !c.same_view(cam)) {
            self.undo.push(cam.clone());
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    pub fn undo(&mut self, current: &Camera) -> Option<Camera> {
        let c = self.undo.pop()?;
        self.redo.push(current.clone());
        Some(c)
    }
    pub fn redo(&mut self, current: &Camera) -> Option<Camera> {
        let c = self.redo.pop()?;
        self.undo.push(current.clone());
        Some(c)
    }
}

impl Camera {
    pub fn same_view(&self, o: &Camera) -> bool {
        (self.target - o.target).length() < 1e-9
            && (self.distance - o.distance).abs() < 1e-9
            && (self.yaw - o.yaw).abs() < 1e-9
            && (self.pitch - o.pitch).abs() < 1e-9
            && (self.ortho_height - o.ortho_height).abs() < 1e-9
            && self.projection == o.projection
    }

    /// Lens length in millimetres (35 mm film equivalent), as shown by ViewportProperties.
    pub fn lens_length(&self) -> f64 {
        // vertical fov on a 24 mm frame height
        12.0 / (self.fov_y * 0.5).tan()
    }

    pub fn set_lens_length(&mut self, mm: f64) {
        let mm = mm.clamp(5.0, 2000.0);
        self.fov_y = 2.0 * (12.0 / mm).atan();
    }

    pub fn ray_at(&self, rect: &ViewRect, x: f64, y: f64) -> Ray {
        self.ray_ndc(rect.screen_to_ndc(x, y), rect.aspect())
    }

    pub fn project(&self, rect: &ViewRect, p: DVec3) -> Option<(DVec2, f64)> {
        let ndc = self.project_ndc(p, rect.aspect())?;
        Some((rect.ndc_to_screen(ndc), ndc.z))
    }
}
