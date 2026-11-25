//! Dynamic window layout: every panel and toolbar group lives in a container that can be docked
//! on any side of the window, floated as its own window, closed and reopened. The layout is
//! persisted with the options and can be saved to / loaded from a file.

use serde::{Deserialize, Serialize};

/// Where a container sits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Placement {
    Left,
    Right,
    Top,
    Bottom,
    Floating { x: f32, y: f32, w: f32, h: f32 },
    Hidden,
}

/// What a container holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerKind {
    /// Tabbed panels (Properties, Layers, ...).
    Panels,
    /// Toolbar groups shown as tabs with a button row.
    Toolbars,
}

/// A tabbed container of panels or toolbar groups.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Container {
    pub id: u32,
    pub kind: ContainerKind,
    pub placement: Placement,
    pub tabs: Vec<String>,
    pub active: usize,
    /// Size along the docking direction (width for left/right, height for top/bottom).
    pub size: f32,
    /// Floating containers can be collapsed to their title bar.
    #[serde(default)]
    pub collapsed: bool,
}

/// All panel names, in their default order.
pub const PANELS: [&str; 8] = ["Properties", "Layers", "Display", "Materials", "Lights", "Rendering", "Help", "Notes"];

/// The complete window layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockLayout {
    pub containers: Vec<Container>,
    pub next_id: u32,
    /// Command-line area, status bar and osnap bar visibility.
    pub show_command_area: bool,
    pub show_status_bar: bool,
    pub show_osnap_bar: bool,
    pub show_viewport_tabs: bool,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self::default_layout()
    }
}

impl DockLayout {
    /// The stock layout: toolbar tabs on top, the Main toolbar on the left, panels on the right.
    pub fn default_layout() -> Self {
        let toolbar_tabs: Vec<String> = crate::ui::toolbars::TAB_NAMES.iter().map(|s| s.to_string()).collect();
        Self {
            containers: vec![
                Container { id: 1, kind: ContainerKind::Toolbars, placement: Placement::Top, tabs: toolbar_tabs, active: 0, size: 64.0, collapsed: false },
                Container { id: 2, kind: ContainerKind::Toolbars, placement: Placement::Left, tabs: vec!["Main".into()], active: 0, size: 70.0, collapsed: false },
                Container { id: 3, kind: ContainerKind::Panels, placement: Placement::Right, tabs: PANELS.iter().map(|s| s.to_string()).collect(), active: 0, size: 300.0, collapsed: false },
            ],
            next_id: 4,
            show_command_area: true,
            show_status_bar: true,
            show_osnap_bar: true,
            show_viewport_tabs: true,
        }
    }

    pub fn container(&self, id: u32) -> Option<&Container> {
        self.containers.iter().find(|c| c.id == id)
    }

    pub fn container_mut(&mut self, id: u32) -> Option<&mut Container> {
        self.containers.iter_mut().find(|c| c.id == id)
    }

    /// Container (and tab index) holding a tab of the given kind.
    pub fn find_tab(&self, kind: ContainerKind, tab: &str) -> Option<(u32, usize)> {
        self.containers.iter().find_map(|c| if c.kind == kind { c.tabs.iter().position(|t| t == tab).map(|i| (c.id, i)) } else { None })
    }

    /// Move a tab out of its container into a new floating container at a screen position.
    pub fn detach(&mut self, kind: ContainerKind, tab: &str, x: f32, y: f32) -> Option<u32> {
        let (cid, idx) = self.find_tab(kind, tab)?;
        let (size, w, h) = {
            let c = self.container(cid)?;
            if c.tabs.len() == 1 && matches!(c.placement, Placement::Floating { .. }) {
                return Some(cid);
            }
            let (w, h) = match kind {
                ContainerKind::Panels => (300.0, 420.0),
                ContainerKind::Toolbars => (420.0, 70.0),
            };
            (c.size, w, h)
        };
        let _ = size;
        self.remove_tab(cid, idx);
        let id = self.next_id;
        self.next_id += 1;
        self.containers.push(Container { id, kind, placement: Placement::Floating { x, y, w, h }, tabs: vec![tab.to_string()], active: 0, size: if kind == ContainerKind::Panels { 300.0 } else { 64.0 }, collapsed: false });
        Some(id)
    }

    fn remove_tab(&mut self, cid: u32, idx: usize) {
        let mut empty = false;
        if let Some(c) = self.container_mut(cid) {
            if idx < c.tabs.len() {
                c.tabs.remove(idx);
            }
            if c.active >= c.tabs.len() {
                c.active = c.tabs.len().saturating_sub(1);
            }
            empty = c.tabs.is_empty();
        }
        if empty {
            self.containers.retain(|c| c.id != cid);
        }
    }

    /// Dock a whole container on a side (merging into an existing docked container of the same
    /// kind on that side when there is one).
    pub fn dock(&mut self, cid: u32, side: Placement) {
        let Some(pos) = self.containers.iter().position(|c| c.id == cid) else { return };
        let kind = self.containers[pos].kind;
        if let Some(target) = self.containers.iter().position(|c| c.id != cid && c.kind == kind && c.placement == side) {
            let moved = self.containers.remove(pos);
            let target = if target > pos { target - 1 } else { target };
            let t = &mut self.containers[target];
            let first = t.tabs.len();
            t.tabs.extend(moved.tabs);
            t.active = first.min(t.tabs.len().saturating_sub(1));
        } else {
            self.containers[pos].placement = side;
        }
    }

    /// Float a docked container at a position.
    pub fn float(&mut self, cid: u32, x: f32, y: f32) {
        if let Some(c) = self.container_mut(cid) {
            let (w, h) = match c.kind {
                ContainerKind::Panels => (300.0, 420.0),
                ContainerKind::Toolbars => (420.0, 70.0),
            };
            c.placement = Placement::Floating { x, y, w, h };
        }
    }

    /// Hide a container (its tabs stay listed so they can be shown again).
    pub fn hide(&mut self, cid: u32) {
        if let Some(c) = self.container_mut(cid) {
            c.placement = Placement::Hidden;
        }
    }

    /// Show a tab: unhide or create its container, and make it active.
    pub fn show_tab(&mut self, kind: ContainerKind, tab: &str) {
        match self.find_tab(kind, tab) {
            Some((cid, idx)) => {
                let default_side = if kind == ContainerKind::Panels { Placement::Right } else { Placement::Top };
                if let Some(c) = self.container_mut(cid) {
                    c.active = idx;
                    c.collapsed = false;
                    if c.placement == Placement::Hidden {
                        c.placement = default_side;
                    }
                }
            }
            None => {
                let side = if kind == ContainerKind::Panels { Placement::Right } else { Placement::Top };
                if let Some(c) = self.containers.iter_mut().find(|c| c.kind == kind && c.placement == side) {
                    c.tabs.push(tab.to_string());
                    c.active = c.tabs.len() - 1;
                } else {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.containers.push(Container { id, kind, placement: side, tabs: vec![tab.to_string()], active: 0, size: if kind == ContainerKind::Panels { 300.0 } else { 64.0 }, collapsed: false });
                }
            }
        }
    }

    /// Open a group as its own floating window (tearing off a flyout). An existing tab of that
    /// name is shown instead; when it sits alone in a container that container is floated.
    pub fn float_group(&mut self, kind: ContainerKind, tab: &str, x: f32, y: f32) -> u32 {
        if let Some((cid, idx)) = self.find_tab(kind, tab) {
            let alone = self.container(cid).is_some_and(|c| c.tabs.len() == 1);
            if alone {
                if let Some(c) = self.container_mut(cid) {
                    c.active = idx;
                    c.collapsed = false;
                    if !matches!(c.placement, Placement::Floating { .. }) {
                        c.placement = Placement::Floating { x, y, w: 260.0, h: 90.0 };
                    }
                }
                return cid;
            }
            return self.detach(kind, tab, x, y).unwrap_or(cid);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.containers.push(Container { id, kind, placement: Placement::Floating { x, y, w: 260.0, h: 90.0 }, tabs: vec![tab.to_string()], active: 0, size: 64.0, collapsed: false });
        id
    }

    /// Whether a tab is currently visible (its container is not hidden and it is the active tab
    /// or the container is a toolbar strip).
    pub fn is_visible(&self, kind: ContainerKind, tab: &str) -> bool {
        self.find_tab(kind, tab).and_then(|(cid, idx)| self.container(cid).map(|c| c.placement != Placement::Hidden && c.active == idx)).unwrap_or(false)
    }

    pub fn containers_at(&self, side: &Placement) -> Vec<u32> {
        self.containers.iter().filter(|c| &c.placement == side).map(|c| c.id).collect()
    }

    pub fn floating(&self) -> Vec<u32> {
        self.containers.iter().filter(|c| matches!(c.placement, Placement::Floating { .. })).map(|c| c.id).collect()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut layout: Self = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        layout.drop_retired_tabs();
        Ok(layout)
    }

    /// Remove toolbar tabs that no longer exist (see `toolbars::RETIRED_TABS`) from a saved
    /// layout, and any toolbar container left empty by that.
    pub fn drop_retired_tabs(&mut self) {
        let retired = crate::ui::toolbars::RETIRED_TABS;
        for c in self.containers.iter_mut().filter(|c| matches!(c.kind, ContainerKind::Toolbars)) {
            c.tabs.retain(|t| !retired.contains(&t.as_str()));
            c.active = c.active.min(c.tabs.len().saturating_sub(1));
        }
        self.containers.retain(|c| !(matches!(c.kind, ContainerKind::Toolbars) && c.tabs.is_empty()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_toolbar_tabs_are_dropped() {
        let mut l = DockLayout::default_layout();
        l.containers[0].tabs.push("New".into());
        l.containers[0].active = l.containers[0].tabs.len() - 1;
        let lone = l.detach(ContainerKind::Toolbars, "Drafting", 50.0, 50.0).expect("detached");
        l.container_mut(lone).unwrap().tabs = vec!["New".into()];
        l.drop_retired_tabs();
        let top = l.container(1).unwrap();
        assert!(!top.tabs.iter().any(|t| t == "New"));
        assert!(top.active < top.tabs.len());
        assert!(l.container(lone).is_none(), "a toolbar container left empty is removed");
    }

    #[test]
    fn detach_dock_and_show() {
        let mut l = DockLayout::default_layout();
        assert_eq!(l.containers.len(), 3);
        let id = l.detach(ContainerKind::Panels, "Layers", 100.0, 100.0).expect("detached");
        assert_eq!(l.containers.len(), 4);
        assert!(matches!(l.container(id).unwrap().placement, Placement::Floating { .. }));
        assert!(!l.container(3).unwrap().tabs.contains(&"Layers".to_string()));
        // dock it back on the right: merges into the right panel container
        l.dock(id, Placement::Right);
        assert_eq!(l.containers.len(), 3);
        let right = l.container(3).unwrap();
        assert_eq!(right.tabs.last().map(|s| s.as_str()), Some("Layers"));
        assert!(l.is_visible(ContainerKind::Panels, "Layers"));
        // detach a toolbar group and dock it at the bottom (new container)
        let t = l.detach(ContainerKind::Toolbars, "Solid Tools", 10.0, 10.0).unwrap();
        l.dock(t, Placement::Bottom);
        assert_eq!(l.containers_at(&Placement::Bottom), vec![t]);
        // hiding and showing a tab
        l.hide(3);
        assert!(!l.is_visible(ContainerKind::Panels, "Layers"));
        l.show_tab(ContainerKind::Panels, "Layers");
        assert!(l.is_visible(ContainerKind::Panels, "Layers"));
        // tearing off a flyout group creates a floating container, a second time reuses it
        let f = l.float_group(ContainerKind::Toolbars, "Array", 50.0, 60.0);
        assert!(matches!(l.container(f).unwrap().placement, Placement::Floating { .. }));
        assert_eq!(l.float_group(ContainerKind::Toolbars, "Array", 0.0, 0.0), f);
        // round trip through JSON
        let text = serde_json::to_string(&l).unwrap();
        let back: DockLayout = serde_json::from_str(&text).unwrap();
        assert_eq!(l, back);
    }
}
