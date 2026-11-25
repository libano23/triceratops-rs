//! Theming: appearance colours of the viewports (Options > Appearance > Colors) plus complete UI
//! themes (Triceratops Light, Triceratops Dark, Blender) that drive the egui widgets and the GPU-drawn viewport
//! backgrounds. Themes are plain JSON files; user themes live in `~/.config/triceratops/themes/`.

use egui::Color32;

pub type Rgb = [u8; 3];

/// Options > Appearance > Colors. Values follow Triceratops 8 defaults as closely as known.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceColors {
    pub background: Rgb,
    pub major_grid: Rgb,
    pub minor_grid: Rgb,
    pub x_axis: Rgb,
    pub y_axis: Rgb,
    pub z_axis: Rgb,
    pub world_x: Rgb,
    pub world_y: Rgb,
    pub world_z: Rgb,
    pub selected: Rgb,
    pub locked: Rgb,
    pub new_layer: Rgb,
    pub feedback: Rgb,
    pub tracking: Rgb,
    pub crosshair: Rgb,
    pub prompt_text: Rgb,
    pub prompt_background: Rgb,
    pub prompt_hover: Rgb,
    pub active_title: Rgb,
    pub inactive_title: Rgb,
}

impl Default for AppearanceColors {
    fn default() -> Self {
        Self {
            background: [140, 140, 140],
            major_grid: [129, 134, 140],
            minor_grid: [147, 153, 160],
            x_axis: [150, 75, 75],
            y_axis: [75, 150, 75],
            z_axis: [0, 0, 150],
            world_x: [255, 0, 0],
            world_y: [0, 255, 0],
            world_z: [0, 0, 255],
            selected: [255, 255, 0],
            locked: [100, 100, 100],
            new_layer: [0, 0, 0],
            feedback: [0, 0, 0],
            tracking: [0, 0, 0],
            crosshair: [0, 0, 0],
            prompt_text: [0, 0, 0],
            prompt_background: [255, 255, 255],
            prompt_hover: [0, 102, 204],
            active_title: [0, 0, 0],
            inactive_title: [96, 96, 96],
        }
    }
}

pub fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn srgb_to_linear(c: [u8; 4]) -> [f32; 4] {
    let f = |v: u8| {
        let x = v as f32 / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(c[0]), f(c[1]), f(c[2]), c[3] as f32 / 255.0]
}

pub fn lin(c: Rgb) -> [f32; 4] {
    srgb_to_linear([c[0], c[1], c[2], 255])
}

pub fn lin_a(c: Rgb, a: f32) -> [f32; 4] {
    let mut l = lin(c);
    l[3] = a;
    l
}

impl AppearanceColors {
    /// Command prompt colours of the dark scheme.
    pub fn set_scheme(&mut self, dark: bool) {
        if dark {
            self.prompt_background = [30, 30, 30];
            self.prompt_text = [230, 230, 230];
            self.prompt_hover = [100, 170, 255];
            self.active_title = [255, 255, 255];
            self.inactive_title = [190, 190, 190];
        } else {
            let d = AppearanceColors::default();
            self.prompt_background = d.prompt_background;
            self.prompt_text = d.prompt_text;
            self.prompt_hover = d.prompt_hover;
            self.active_title = d.active_title;
            self.inactive_title = d.inactive_title;
        }
    }
}

// ------------------------------------------------------------------------------------------------
// UI themes

/// Colours and shapes of the widgets (everything egui draws).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UiColors {
    /// Window background: visible in the gaps between regions (Blender's dark editor gaps).
    pub window_bg: Rgb,
    /// Fill of panels / editor regions.
    pub panel_bg: Rgb,
    /// Headers (menu bar, toolbar tab strip, panel titles).
    pub header_bg: Rgb,
    /// Buttons and inputs at rest.
    pub widget_bg: Rgb,
    pub widget_hover: Rgb,
    /// Pressed / toggled widgets.
    pub widget_active: Rgb,
    /// Highlight colour (selected items, option toggles, text selection).
    pub accent: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    pub text_on_accent: Rgb,
    pub border: Rgb,
    pub popup_bg: Rgb,
    /// Text input fields.
    pub field_bg: Rgb,
    pub tab_active: Rgb,
    pub tab_inactive: Rgb,
    pub viewport_border: Rgb,
    pub tooltip_bg: Rgb,
    pub tooltip_text: Rgb,
    /// Corner radius of widgets in points.
    pub rounding: u8,
    /// Corner radius of the editor regions (panels, viewports).
    pub region_rounding: u8,
    /// Gap between regions in points (0 = Triceratops look, 3-4 = Blender look).
    pub region_gap: f32,
    pub font_size: f32,
}

impl Default for UiColors {
    fn default() -> Self {
        Theme::default_light().ui
    }
}

/// A complete theme: widgets plus viewport colours and the GPU background gradient.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub ui: UiColors,
    pub viewport: AppearanceColors,
    /// Optional viewport background gradient (top, bottom) rendered by the viewport shader.
    pub viewport_gradient: Option<(Rgb, Rgb)>,
    /// Gumball axis colours.
    pub gumball: [Rgb; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Theme::default_light()
    }
}

impl Theme {
    pub fn default_light() -> Self {
        Self {
            name: "Triceratops Light".into(),
            dark: false,
            ui: UiColors {
                window_bg: [240, 240, 240],
                panel_bg: [240, 240, 240],
                header_bg: [240, 240, 240],
                widget_bg: [228, 228, 228],
                widget_hover: [205, 222, 243],
                widget_active: [170, 200, 240],
                accent: [0, 120, 215],
                text: [0, 0, 0],
                text_dim: [96, 96, 96],
                text_on_accent: [255, 255, 255],
                border: [190, 190, 190],
                popup_bg: [246, 246, 246],
                field_bg: [255, 255, 255],
                tab_active: [255, 255, 255],
                tab_inactive: [214, 214, 214],
                viewport_border: [90, 90, 90],
                tooltip_bg: [255, 255, 225],
                tooltip_text: [0, 0, 0],
                rounding: 2,
                region_rounding: 0,
                region_gap: 0.0,
                font_size: 13.0,
            },
            viewport: AppearanceColors::default(),
            viewport_gradient: None,
            gumball: [[230, 60, 60], [60, 190, 60], [70, 110, 240]],
        }
    }

    pub fn default_dark() -> Self {
        let mut viewport = AppearanceColors::default();
        viewport.set_scheme(true);
        Self {
            name: "Triceratops Dark".into(),
            dark: true,
            ui: UiColors {
                window_bg: [37, 37, 38],
                panel_bg: [45, 45, 48],
                header_bg: [45, 45, 48],
                widget_bg: [62, 62, 66],
                widget_hover: [70, 90, 120],
                widget_active: [0, 100, 190],
                accent: [0, 100, 190],
                text: [230, 230, 230],
                text_dim: [170, 170, 170],
                text_on_accent: [255, 255, 255],
                border: [70, 70, 74],
                popup_bg: [37, 37, 38],
                field_bg: [30, 30, 30],
                tab_active: [70, 70, 74],
                tab_inactive: [52, 52, 55],
                viewport_border: [90, 90, 90],
                tooltip_bg: [255, 255, 225],
                tooltip_text: [0, 0, 0],
                rounding: 2,
                region_rounding: 0,
                region_gap: 0.0,
                font_size: 13.0,
            },
            viewport,
            viewport_gradient: None,
            gumball: [[230, 60, 60], [60, 190, 60], [70, 110, 240]],
        }
    }

    /// Blender 4.x "Blender Dark" look: dark editor gaps, rounded regions, grey widgets with a
    /// blue accent, orange selection and Blender's axis colours in the viewport.
    pub fn blender() -> Self {
        Self {
            name: "Blender".into(),
            dark: true,
            ui: UiColors {
                window_bg: [22, 22, 22],
                panel_bg: [48, 48, 48],
                header_bg: [48, 48, 48],
                widget_bg: [84, 84, 84],
                widget_hover: [101, 101, 101],
                widget_active: [71, 114, 179],
                accent: [71, 114, 179],
                text: [230, 230, 230],
                text_dim: [154, 154, 154],
                text_on_accent: [255, 255, 255],
                border: [61, 61, 61],
                popup_bg: [24, 24, 24],
                field_bg: [29, 29, 29],
                tab_active: [61, 61, 61],
                tab_inactive: [35, 35, 35],
                viewport_border: [22, 22, 22],
                tooltip_bg: [24, 24, 24],
                tooltip_text: [230, 230, 230],
                rounding: 4,
                region_rounding: 6,
                region_gap: 3.0,
                font_size: 13.0,
            },
            viewport: AppearanceColors {
                background: [61, 61, 61],
                major_grid: [86, 86, 86],
                minor_grid: [72, 72, 72],
                x_axis: [255, 51, 82],
                y_axis: [139, 220, 0],
                z_axis: [40, 144, 255],
                world_x: [255, 51, 82],
                world_y: [139, 220, 0],
                world_z: [40, 144, 255],
                selected: [255, 160, 40],
                locked: [120, 120, 120],
                new_layer: [230, 230, 230],
                feedback: [255, 255, 255],
                tracking: [255, 255, 255],
                crosshair: [230, 230, 230],
                prompt_text: [230, 230, 230],
                prompt_background: [29, 29, 29],
                prompt_hover: [71, 114, 179],
                active_title: [255, 255, 255],
                inactive_title: [154, 154, 154],
            },
            viewport_gradient: Some(([61, 61, 61], [48, 48, 48])),
            gumball: [[255, 51, 82], [139, 220, 0], [40, 144, 255]],
        }
    }

    pub fn builtin(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace(' ', "").as_str() {
            "triceratopslight" | "light" => Some(Self::default_light()),
            "triceratopsdark" | "dark" => Some(Self::default_dark()),
            "blender" | "blenderdark" => Some(Self::blender()),
            _ => None,
        }
    }

    /// Folder with user theme files.
    pub fn user_dir() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("triceratops").join("themes"))
    }

    /// Built-in themes followed by the user themes found on disk.
    pub fn all() -> Vec<Theme> {
        let mut out = vec![Self::default_light(), Self::default_dark(), Self::blender()];
        if let Some(dir) = Self::user_dir() {
            if let Ok(rd) = std::fs::read_dir(dir) {
                let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
                files.sort();
                for f in files {
                    if let Ok(t) = Self::load(&f) {
                        if !out.iter().any(|o| o.name.eq_ignore_ascii_case(&t.name)) {
                            out.push(t);
                        }
                    }
                }
            }
        }
        out
    }

    pub fn by_name(name: &str) -> Option<Self> {
        Self::builtin(name).or_else(|| Self::all().into_iter().find(|t| t.name.eq_ignore_ascii_case(name.trim())))
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut t: Theme = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if t.name.trim().is_empty() {
            t.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Theme".into());
        }
        Ok(t)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Save into the user theme folder under the theme name.
    pub fn save_user(&self) -> Result<std::path::PathBuf, String> {
        let dir = Self::user_dir().ok_or("no config directory")?;
        let file = dir.join(format!("{}.json", self.name.replace(['/', '\\'], "_")));
        self.save(&file)?;
        Ok(file)
    }

    /// Clear colour of the window (the surface is cleared by the GPU before egui draws).
    pub fn clear_color(&self) -> [f64; 3] {
        let l = lin(self.ui.window_bg);
        [l[0] as f64, l[1] as f64, l[2] as f64]
    }

    /// Frame used by the bars and panels: flat for the Triceratops themes, rounded and separated by a
    /// gap for Blender-like themes.
    pub fn region_frame(&self) -> egui::Frame {
        let gap = self.ui.region_gap;
        egui::Frame::new()
            .fill(c32(self.ui.panel_bg))
            .corner_radius(egui::CornerRadius::same(self.ui.region_rounding))
            .outer_margin(egui::Margin::same(gap.round() as i8))
            .inner_margin(egui::Margin::symmetric(6, 3))
    }

    pub fn header_frame(&self) -> egui::Frame {
        self.region_frame().fill(c32(self.ui.header_bg))
    }

    /// egui visuals for this theme.
    pub fn visuals(&self) -> egui::Visuals {
        let u = &self.ui;
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        let r = egui::CornerRadius::same(u.rounding);
        v.dark_mode = self.dark;
        v.panel_fill = c32(u.panel_bg);
        v.window_fill = c32(u.popup_bg);
        v.extreme_bg_color = c32(u.field_bg);
        v.faint_bg_color = c32(u.widget_bg);
        v.code_bg_color = c32(u.field_bg);
        v.window_stroke = egui::Stroke::new(1.0_f32, c32(u.border));
        v.widgets.noninteractive.bg_fill = c32(u.panel_bg);
        v.widgets.noninteractive.weak_bg_fill = c32(u.panel_bg);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, c32(u.border));
        v.widgets.noninteractive.fg_stroke.color = c32(u.text);
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.inactive.bg_fill = c32(u.widget_bg);
        v.widgets.inactive.weak_bg_fill = c32(u.panel_bg);
        v.widgets.inactive.bg_stroke = egui::Stroke::new(if u.region_gap > 0.0 { 1.0_f32 } else { 0.0_f32 }, c32(u.border));
        v.widgets.inactive.fg_stroke.color = c32(u.text);
        v.widgets.inactive.corner_radius = r;
        v.widgets.hovered.bg_fill = c32(u.widget_hover);
        v.widgets.hovered.weak_bg_fill = c32(u.widget_hover);
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, if u.region_gap > 0.0 { c32(u.widget_hover) } else { c32(u.accent) });
        // egui's "strong" text colour is the active widget text: keep it readable on light themes
        let emph = if self.dark { u.text_on_accent } else { u.text };
        v.widgets.hovered.fg_stroke.color = c32(emph);
        v.widgets.hovered.corner_radius = r;
        v.widgets.active.bg_fill = c32(u.widget_active);
        v.widgets.active.weak_bg_fill = c32(u.widget_active);
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, c32(u.accent));
        v.widgets.active.fg_stroke.color = c32(emph);
        v.widgets.active.corner_radius = r;
        v.widgets.open.bg_fill = c32(u.widget_active);
        v.widgets.open.weak_bg_fill = c32(u.widget_active);
        v.widgets.open.fg_stroke.color = c32(emph);
        v.widgets.open.corner_radius = r;
        v.selection.bg_fill = c32(u.accent);
        v.selection.stroke = egui::Stroke::new(1.0_f32, c32(u.text_on_accent));
        v.hyperlink_color = c32(u.accent);
        v.override_text_color = None;
        v.window_shadow = egui::epaint::Shadow::NONE;
        v.popup_shadow = egui::epaint::Shadow::NONE;
        v.window_corner_radius = egui::CornerRadius::same(u.rounding.max(3));
        v.menu_corner_radius = egui::CornerRadius::same(if u.region_gap > 0.0 { u.rounding } else { 0 });
        v.text_cursor.stroke.color = c32(u.text);
        v
    }
}

/// Apply a theme to an egui context (visuals, spacing and text sizes).
pub fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    ctx.set_visuals(theme.visuals());
    let mut style = (*ctx.global_style()).clone();
    let s = theme.ui.font_size;
    style.spacing.item_spacing = egui::vec2(4.0, 3.0);
    style.spacing.button_padding = if theme.ui.region_gap > 0.0 { egui::vec2(7.0, 3.0) } else { egui::vec2(5.0, 2.0) };
    style.spacing.menu_margin = egui::Margin::same(4);
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(s));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(s));
    style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(s));
    style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional((s - 2.0).max(8.0)));
    style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(s + 2.0));
    ctx.set_global_style(style);
}

/// Paint the four corner masks that make a rectangular region look rounded (Blender editors).
pub fn round_region_corners(painter: &egui::Painter, rect: egui::Rect, radius: f32, gap_color: Color32) {
    if radius < 0.5 {
        return;
    }
    let n = 8;
    let corners = [(rect.left_top(), 1.0, 1.0), (rect.right_top(), -1.0, 1.0), (rect.right_bottom(), -1.0, -1.0), (rect.left_bottom(), 1.0, -1.0)];
    for (c, sx, sy) in corners {
        let centre = egui::pos2(c.x + sx * radius, c.y + sy * radius);
        let mut pts = vec![c];
        for k in 0..=n {
            // arc from the horizontal tangent point to the vertical one, around the centre
            let a = std::f32::consts::FRAC_PI_2 * k as f32 / n as f32;
            let (dx, dy) = (-(sx) * radius * a.cos(), -(sy) * radius * a.sin());
            pts.push(egui::pos2(centre.x + dx, centre.y + dy));
        }
        painter.add(egui::Shape::convex_polygon(pts, gap_color, egui::Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_roundtrip_json() {
        for name in ["Triceratops Light", "Triceratops Dark", "Blender"] {
            let t = Theme::by_name(name).expect(name);
            let text = serde_json::to_string(&t).unwrap();
            let back: Theme = serde_json::from_str(&text).unwrap();
            assert_eq!(t, back);
        }
        assert!(Theme::builtin("blender").unwrap().dark);
        assert!(!Theme::builtin("Triceratops Light").unwrap().dark);
        assert!(Theme::builtin("nope").is_none());
    }

    #[test]
    fn partial_theme_file_uses_defaults() {
        let t: Theme = serde_json::from_str(r#"{"name":"Mine","dark":true,"ui":{"accent":[1,2,3]}}"#).unwrap();
        assert_eq!(t.ui.accent, [1, 2, 3]);
        assert_eq!(t.ui.panel_bg, Theme::default_light().ui.panel_bg);
        assert_eq!(t.viewport, AppearanceColors::default());
    }

    #[test]
    fn blender_theme_drives_egui_visuals() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, &Theme::blender());
        let v = ctx.global_style().visuals.clone();
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, Color32::from_rgb(48, 48, 48));
        assert_eq!(v.selection.bg_fill, Color32::from_rgb(71, 114, 179));
        assert_eq!(v.widgets.inactive.corner_radius, egui::CornerRadius::same(4));
        apply_theme(&ctx, &Theme::default_light());
        assert!(!ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.widgets.inactive.corner_radius, egui::CornerRadius::same(2));
    }

    #[test]
    fn user_theme_save_and_load() {
        let dir = std::env::temp_dir().join(format!("triceratops-theme-test-{}", std::process::id()));
        let mut t = Theme::blender();
        t.name = "My Blender".into();
        t.ui.accent = [200, 100, 0];
        let p = dir.join("my.json");
        t.save(&p).unwrap();
        let back = Theme::load(&p).unwrap();
        assert_eq!(back.name, "My Blender");
        assert_eq!(back.ui.accent, [200, 100, 0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_color_is_linear_window_background() {
        let t = Theme::blender();
        let c = t.clear_color();
        assert!(c[0] < 0.02 && c[0] > 0.0, "{c:?}");
        let l = Theme::default_light().clear_color();
        assert!(l[0] > 0.8);
    }
}
