//! The node graph: nodes, ports, wires, groups, comments and clusters.
//!
//! The graph is plain data. It knows nothing of geometry or of the canvas; it stores which
//! component a node is, where it sits, what inline values its unconnected inputs carry,
//! which output feeds which input, and — for clusters — a whole nested graph behind one
//! node. The JSON form is the one the Python Locust writes (`.locust` files).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::{json, Map, Value as J};

use super::components::registry;

/// A short random id (8 hex digits), unique enough for one graph.
pub fn new_uid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    // splitmix64 of time and counter
    let mut z = t ^ c.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((std::process::id() as u64) << 32);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    format!("{:08x}", z as u32)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub uid: String,
    /// Component name, or `Cluster:<id>`.
    pub kind: String,
    pub x: f64,
    pub y: f64,
    /// Input name (or index as text) -> inline value, as JSON.
    pub values: Map<String, J>,
    pub nickname: Option<String>,
    pub group: Option<String>,
    /// Bypassed: passes its inputs on.
    pub disabled: bool,
}

impl Node {
    pub fn label(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub src: String,
    pub src_port: usize,
    pub dst: String,
    pub dst_port: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub uid: String,
    pub name: String,
    pub color: String,
    pub nodes: Vec<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub uid: String,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub color: String,
}

/// Ports of a cluster node: (name, kind) pairs.
pub type Ports = (Vec<(String, String)>, Vec<(String, String)>);

/// How a port of a Script node receives its data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    /// One item at a time, matched like any component's inputs.
    #[default]
    Item,
    /// The whole branch as a list.
    List,
    /// The whole tree, as a list of branches.
    Tree,
}

impl Access {
    fn parse(s: &str) -> Self {
        match s {
            "list" => Access::List,
            "tree" => Access::Tree,
            _ => Access::Item,
        }
    }
}

/// A port of a node that declares its own ports (Script, Expression).
#[derive(Clone, Debug, PartialEq)]
pub struct PortDef {
    pub name: String,
    pub kind: String,
    pub access: Access,
}

/// Nodes whose ports are part of the node (`_inputs`, `_outputs` in its values).
pub fn is_dynamic(kind: &str) -> bool {
    kind == "Script" || kind == "Expression"
}

/// Ports from `_inputs` / `_outputs`: entries are names or `{name, kind, access}`.
fn port_defs(v: Option<&J>) -> Option<Vec<PortDef>> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|d| match d {
                J::String(name) => Some(PortDef { name: name.clone(), kind: "any".into(), access: Access::Item }),
                J::Object(o) => Some(PortDef {
                    name: o.get("name")?.as_str()?.to_string(),
                    kind: o.get("kind").and_then(|k| k.as_str()).unwrap_or("any").to_string(),
                    access: Access::parse(o.get("access").and_then(|a| a.as_str()).unwrap_or("item")),
                }),
                _ => None,
            })
            .collect(),
    )
}

fn default_ports(names: &[&str]) -> Vec<PortDef> {
    names.iter().map(|n| PortDef { name: n.to_string(), kind: "any".into(), access: Access::Item }).collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub groups: Vec<Group>,
    pub comments: Vec<Comment>,
    /// Cluster id -> definition `{name, graph, inputs, outputs}`.
    pub clusters: BTreeMap<String, J>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node(&self, uid: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.uid == uid)
    }

    pub fn node_mut(&mut self, uid: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.uid == uid)
    }

    pub fn contains(&self, uid: &str) -> bool {
        self.node(uid).is_some()
    }

    // ---------------------------------------------------------------- editing

    pub fn add_node(&mut self, kind: &str, x: f64, y: f64) -> String {
        self.add_node_with(kind, x, y, Map::new())
    }

    pub fn add_node_with(&mut self, kind: &str, x: f64, y: f64, values: Map<String, J>) -> String {
        let uid = new_uid();
        self.nodes.push(Node { uid: uid.clone(), kind: kind.to_string(), x, y, values, nickname: None, group: None, disabled: false });
        uid
    }

    /// Set an inline input value.
    pub fn set_value(&mut self, uid: &str, name: &str, value: J) {
        if let Some(n) = self.node_mut(uid) {
            n.values.insert(name.to_string(), value);
        }
    }

    pub fn remove_node(&mut self, uid: &str) {
        self.nodes.retain(|n| n.uid != uid);
        self.connections.retain(|c| c.src != uid && c.dst != uid);
        for g in &mut self.groups {
            g.nodes.retain(|n| n != uid);
        }
    }

    /// Wire `src` output `src_port` into `dst` input `dst_port`. One wire per input (a
    /// new one replaces the old); refused when it would close a loop.
    pub fn connect(&mut self, src: &str, src_port: usize, dst: &str, dst_port: usize) -> bool {
        if !self.contains(src) || !self.contains(dst) || src == dst {
            return false;
        }
        if self.reaches(dst, src) {
            return false;
        }
        self.connections.retain(|c| !(c.dst == dst && c.dst_port == dst_port));
        self.connections.push(Connection { src: src.to_string(), src_port, dst: dst.to_string(), dst_port });
        true
    }

    pub fn disconnect(&mut self, dst: &str, dst_port: usize) {
        self.connections.retain(|c| !(c.dst == dst && c.dst_port == dst_port));
    }

    pub fn source_of(&self, dst: &str, dst_port: usize) -> Option<&Connection> {
        self.connections.iter().find(|c| c.dst == dst && c.dst_port == dst_port)
    }

    /// True if following wires from `start` gets to `target`.
    fn reaches(&self, start: &str, target: &str) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(n) = stack.pop() {
            if n == target {
                return true;
            }
            if !seen.insert(n.clone()) {
                continue;
            }
            stack.extend(self.connections.iter().filter(|c| c.src == n).map(|c| c.dst.clone()));
        }
        false
    }

    // ---------------------------------------------------------------- groups

    pub fn add_group(&mut self, nodes: &[String], name: &str) -> String {
        let uid = new_uid();
        let members: Vec<String> = nodes.iter().filter(|n| self.contains(n)).cloned().collect();
        for m in &members {
            if let Some(n) = self.node_mut(m) {
                n.group = Some(uid.clone());
            }
        }
        self.groups.push(Group { uid: uid.clone(), name: name.to_string(), color: "#4a3f28".into(), nodes: members, x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        uid
    }

    pub fn remove_group(&mut self, uid: &str) {
        if let Some(g) = self.groups.iter().find(|g| g.uid == uid).cloned() {
            for m in &g.nodes {
                if let Some(n) = self.node_mut(m) {
                    if n.group.as_deref() == Some(uid) {
                        n.group = None;
                    }
                }
            }
        }
        self.groups.retain(|g| g.uid != uid);
    }

    pub fn add_comment(&mut self, x: f64, y: f64, text: &str) -> String {
        let uid = new_uid();
        self.comments.push(Comment { uid: uid.clone(), text: text.to_string(), x, y, w: 220.0, h: 80.0, color: "#2b2c30".into() });
        uid
    }

    pub fn remove_comment(&mut self, uid: &str) {
        self.comments.retain(|c| c.uid != uid);
    }

    /// Copy nodes (and the wires wholly inside them), offset a little. Returns new uids.
    pub fn duplicate(&mut self, uids: &[String], dx: f64, dy: f64) -> Vec<String> {
        let keep: Vec<String> = uids.iter().filter(|u| self.contains(u)).cloned().collect();
        let mut mapping: HashMap<String, String> = HashMap::new();
        for uid in &keep {
            let old = self.node(uid).unwrap().clone();
            let new = self.add_node_with(&old.kind, old.x + dx, old.y + dy, old.values.clone());
            if let Some(n) = self.node_mut(&new) {
                n.nickname = old.nickname.clone();
                n.disabled = old.disabled;
            }
            mapping.insert(uid.clone(), new);
        }
        for c in self.connections.clone() {
            if let (Some(s), Some(d)) = (mapping.get(&c.src), mapping.get(&c.dst)) {
                self.connect(s, c.src_port, d, c.dst_port);
            }
        }
        keep.iter().map(|u| mapping[u].clone()).collect()
    }

    // --------------------------------------------------------------- clusters

    pub fn is_cluster(&self, node: &Node) -> bool {
        node.kind.starts_with("Cluster:")
    }

    pub fn cluster_of(&self, node: &Node) -> Option<&J> {
        node.kind.strip_prefix("Cluster:").and_then(|id| self.clusters.get(id))
    }


    /// Inputs and outputs of a Script or Expression node.
    pub fn dynamic_ports(&self, node: &Node) -> (Vec<PortDef>, Vec<PortDef>) {
        let (ins, outs): (&[&str], &[&str]) = if node.kind == "Expression" { (&["x", "y", "z"], &["Result"]) } else { (&["x", "y"], &["a"]) };
        (port_defs(node.values.get("_inputs")).unwrap_or_else(|| default_ports(ins)), port_defs(node.values.get("_outputs")).unwrap_or_else(|| default_ports(outs)))
    }

    fn set_dynamic_ports(&mut self, uid: &str, out: bool, ports: &[PortDef]) {
        let key = if out { "_outputs" } else { "_inputs" };
        let v: Vec<J> = ports
            .iter()
            .map(|d| {
                if out {
                    json!({ "name": d.name })
                } else {
                    json!({ "name": d.name, "kind": d.kind, "access": match d.access { Access::Item => "item", Access::List => "list", Access::Tree => "tree" } })
                }
            })
            .collect();
        if let Some(n) = self.node_mut(uid) {
            n.values.insert(key.into(), J::Array(v));
        }
    }

    fn ports_side(&self, uid: &str, out: bool) -> Option<Vec<PortDef>> {
        let n = self.node(uid).filter(|n| is_dynamic(&n.kind))?;
        let (ins, outs) = self.dynamic_ports(n);
        Some(if out { outs } else { ins })
    }

    /// Add a port to a Script/Expression node; returns its name (x, y, z, u, v, w… for
    /// inputs, a, b, c… for outputs, then numbered).
    pub fn add_port(&mut self, uid: &str, out: bool) -> Option<String> {
        let mut ports = self.ports_side(uid, out)?;
        let pool: &[&str] = if out { &["a", "b", "c", "d", "e", "f", "g", "h"] } else { &["x", "y", "z", "u", "v", "w", "t", "s"] };
        let taken = |n: &str| ports.iter().any(|p| p.name == n);
        let name = pool.iter().find(|n| !taken(n)).map(|n| n.to_string()).unwrap_or_else(|| (1..).map(|i| format!("{}{i}", if out { "out" } else { "in" })).find(|n| !taken(n)).unwrap_or_default());
        ports.push(PortDef { name: name.clone(), kind: "any".into(), access: Access::Item });
        self.set_dynamic_ports(uid, out, &ports);
        Some(name)
    }

    /// Rename a port; false when the name is taken or is not an identifier.
    pub fn rename_port(&mut self, uid: &str, out: bool, index: usize, name: &str) -> bool {
        let Some(mut ports) = self.ports_side(uid, out) else { return false };
        let ident = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ident || index >= ports.len() || ports.iter().enumerate().any(|(i, p)| i != index && p.name == name) {
            return false;
        }
        ports[index].name = name.to_string();
        self.set_dynamic_ports(uid, out, &ports);
        true
    }

    /// Set how an input port of a Script receives its data.
    pub fn set_port_access(&mut self, uid: &str, index: usize, access: Access) {
        if let Some(mut ports) = self.ports_side(uid, false) {
            if let Some(p) = ports.get_mut(index) {
                p.access = access;
                self.set_dynamic_ports(uid, false, &ports);
            }
        }
    }

    /// Remove a port; its wires go and the wires of the ports after it move up one.
    pub fn remove_port(&mut self, uid: &str, out: bool, index: usize) {
        let Some(mut ports) = self.ports_side(uid, out) else { return };
        if index >= ports.len() {
            return;
        }
        ports.remove(index);
        self.set_dynamic_ports(uid, out, &ports);
        self.connections.retain(|c| !(if out { c.src == uid && c.src_port == index } else { c.dst == uid && c.dst_port == index }));
        for c in self.connections.iter_mut() {
            if out && c.src == uid && c.src_port > index {
                c.src_port -= 1;
            }
            if !out && c.dst == uid && c.dst_port > index {
                c.dst_port -= 1;
            }
        }
    }

    /// (inputs, outputs) of any node as (name, kind) pairs: a component's, a cluster's or a
    /// Script's own. Unknown kinds have none.
    pub fn node_ports(&self, node: &Node) -> Ports {
        if is_dynamic(&node.kind) {
            let (ins, outs) = self.dynamic_ports(node);
            let pairs = |v: Vec<PortDef>| v.into_iter().map(|d| (d.name, d.kind)).collect();
            return (pairs(ins), pairs(outs));
        }
        if self.is_cluster(node) {
            return self.cluster_ports(node);
        }
        match super::components::registry().get(&node.kind) {
            Some(c) => (c.inputs.iter().map(|i| (i.name.to_string(), i.kind.name().to_string())).collect(), c.outputs.iter().map(|o| (o.to_string(), "any".to_string())).collect()),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// (inputs, outputs) of a cluster node, as (name, kind) pairs.
    pub fn cluster_ports(&self, node: &Node) -> Ports {
        let Some(def) = self.cluster_of(node) else { return (Vec::new(), Vec::new()) };
        let ports = |key: &str| -> Vec<(String, String)> {
            def.get(key)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().map(|d| (d.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(), d.get("kind").and_then(|x| x.as_str()).unwrap_or("any").to_string())).collect())
                .unwrap_or_default()
        };
        (ports("inputs"), ports("outputs"))
    }

    /// Collapse a selection into one node backed by a nested graph. Wires crossing the
    /// selection become the cluster's ports: inside, each becomes a Cluster Input / Cluster
    /// Output proxy so the nested graph solves on its own. Returns the cluster node's uid.
    pub fn make_cluster(&mut self, uids: &[String], name: &str) -> Option<String> {
        let keep: Vec<String> = uids.iter().filter(|u| self.contains(u)).cloned().collect();
        if keep.is_empty() {
            return None;
        }
        let keep_set: HashSet<&String> = keep.iter().collect();
        let cluster_id = new_uid();
        let mut inner = Graph::new();
        let mut inner_map: HashMap<String, String> = HashMap::new();
        for uid in &keep {
            let old = self.node(uid).unwrap().clone();
            let new = inner.add_node_with(&old.kind, old.x, old.y, old.values.clone());
            if let Some(n) = inner.node_mut(&new) {
                n.nickname = old.nickname.clone();
                n.disabled = old.disabled;
            }
            inner_map.insert(uid.clone(), new);
        }
        for c in &self.connections {
            if keep_set.contains(&c.src) && keep_set.contains(&c.dst) {
                inner.connect(&inner_map[&c.src], c.src_port, &inner_map[&c.dst], c.dst_port);
            }
        }
        let reg = registry();
        let boundary_in: Vec<Connection> = self.connections.iter().filter(|c| keep_set.contains(&c.dst) && !keep_set.contains(&c.src)).cloned().collect();
        let boundary_out: Vec<Connection> = self.connections.iter().filter(|c| keep_set.contains(&c.src) && !keep_set.contains(&c.dst)).cloned().collect();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for c in &boundary_in {
            let label = self.node(&c.dst).and_then(|n| reg.get(&n.kind)).and_then(|comp| comp.inputs.get(c.dst_port)).map(|i| i.name.to_string()).unwrap_or_else(|| "In".into());
            let mut values = Map::new();
            values.insert("Name".into(), json!(label));
            let proxy = inner.add_node_with("Cluster Input", 0.0, 0.0, values);
            inner.connect(&proxy, 0, &inner_map[&c.dst], c.dst_port);
            inputs.push(json!({"node": proxy, "name": label, "kind": "any"}));
        }
        for c in &boundary_out {
            let label = self.node(&c.src).and_then(|n| reg.get(&n.kind)).and_then(|comp| comp.outputs.get(c.src_port)).map(|o| o.to_string()).unwrap_or_else(|| "Out".into());
            let mut values = Map::new();
            values.insert("Name".into(), json!(label));
            let proxy = inner.add_node_with("Cluster Output", 0.0, 0.0, values);
            inner.connect(&inner_map[&c.src], c.src_port, &proxy, 0);
            outputs.push(json!({"node": proxy, "name": label, "kind": "any"}));
        }
        self.clusters.insert(cluster_id.clone(), json!({"name": name, "graph": inner.to_json(), "inputs": inputs, "outputs": outputs}));
        let (mut cx, mut cy) = (0.0, 0.0);
        for uid in &keep {
            let n = self.node(uid).unwrap();
            cx += n.x;
            cy += n.y;
        }
        cx /= keep.len() as f64;
        cy /= keep.len() as f64;
        for uid in &keep {
            self.remove_node(uid);
        }
        let cluster_uid = new_uid();
        self.nodes.push(Node { uid: cluster_uid.clone(), kind: format!("Cluster:{cluster_id}"), x: cx, y: cy, values: Map::new(), nickname: Some(name.to_string()), group: None, disabled: false });
        for (i, c) in boundary_in.iter().enumerate() {
            self.connect(&c.src, c.src_port, &cluster_uid, i);
        }
        for (i, c) in boundary_out.iter().enumerate() {
            self.connect(&cluster_uid, i, &c.dst, c.dst_port);
        }
        Some(cluster_uid)
    }

    // -------------------------------------------------------------- traversal

    /// Node uids, inputs before the nodes that read them.
    pub fn topological(&self) -> Vec<String> {
        let mut indeg: HashMap<&str, usize> = self.nodes.iter().map(|n| (n.uid.as_str(), 0)).collect();
        for c in &self.connections {
            if indeg.contains_key(c.src.as_str()) {
                if let Some(d) = indeg.get_mut(c.dst.as_str()) {
                    *d += 1;
                }
            }
        }
        let mut ready: VecDeque<&str> = self.nodes.iter().map(|n| n.uid.as_str()).filter(|u| indeg[u] == 0).collect();
        let mut order: Vec<String> = Vec::new();
        while let Some(uid) = ready.pop_front() {
            order.push(uid.to_string());
            for c in &self.connections {
                if c.src == uid {
                    if let Some(d) = indeg.get_mut(c.dst.as_str()) {
                        *d -= 1;
                        if *d == 0 {
                            ready.push_back(c.dst.as_str());
                        }
                    }
                }
            }
        }
        for n in &self.nodes {
            if !order.contains(&n.uid) {
                order.push(n.uid.clone());
            }
        }
        order
    }

    // ------------------------------------------------------------ persistence

    pub fn to_json(&self) -> J {
        json!({
            "nodes": self.nodes.iter().map(|n| json!({
                "uid": n.uid, "type": n.kind, "x": n.x, "y": n.y, "values": n.values,
                "nickname": n.nickname, "group": n.group, "disabled": n.disabled,
            })).collect::<Vec<_>>(),
            "connections": self.connections.iter().map(|c| json!({
                "src": c.src, "src_port": c.src_port, "dst": c.dst, "dst_port": c.dst_port,
            })).collect::<Vec<_>>(),
            "groups": self.groups.iter().map(|g| json!({
                "uid": g.uid, "name": g.name, "color": g.color, "nodes": g.nodes,
                "x": g.x, "y": g.y, "w": g.w, "h": g.h,
            })).collect::<Vec<_>>(),
            "comments": self.comments.iter().map(|c| json!({
                "uid": c.uid, "text": c.text, "x": c.x, "y": c.y, "w": c.w, "h": c.h, "color": c.color,
            })).collect::<Vec<_>>(),
            "clusters": self.clusters,
        })
    }

    pub fn from_json(data: &J) -> Graph {
        let s = |v: &J, k: &str, d: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or(d).to_string();
        let f = |v: &J, k: &str, d: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(d);
        let arr = |k: &str| data.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let mut g = Graph::new();
        for n in arr("nodes") {
            let Some(kind) = n.get("type").and_then(|x| x.as_str()) else { continue };
            let uid = n.get("uid").and_then(|x| x.as_str()).filter(|u| !u.is_empty()).map(String::from).unwrap_or_else(new_uid);
            g.nodes.push(Node {
                uid,
                kind: kind.to_string(),
                x: f(&n, "x", 0.0),
                y: f(&n, "y", 0.0),
                values: n.get("values").and_then(|v| v.as_object()).cloned().unwrap_or_default(),
                nickname: n.get("nickname").and_then(|x| x.as_str()).map(String::from),
                group: n.get("group").and_then(|x| x.as_str()).map(String::from),
                disabled: n.get("disabled").and_then(|x| x.as_bool()).unwrap_or(false),
            });
        }
        for c in arr("connections") {
            let (src, dst) = (s(&c, "src", ""), s(&c, "dst", ""));
            if g.contains(&src) && g.contains(&dst) {
                let port = |k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                g.connections.push(Connection { src, src_port: port("src_port"), dst, dst_port: port("dst_port") });
            }
        }
        for gr in arr("groups") {
            g.groups.push(Group {
                uid: Some(s(&gr, "uid", "")).filter(|u| !u.is_empty()).unwrap_or_else(new_uid),
                name: s(&gr, "name", "Group"),
                color: s(&gr, "color", "#4a3f28"),
                nodes: gr.get("nodes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
                x: f(&gr, "x", 0.0),
                y: f(&gr, "y", 0.0),
                w: f(&gr, "w", 0.0),
                h: f(&gr, "h", 0.0),
            });
        }
        for cm in arr("comments") {
            g.comments.push(Comment {
                uid: Some(s(&cm, "uid", "")).filter(|u| !u.is_empty()).unwrap_or_else(new_uid),
                text: s(&cm, "text", ""),
                x: f(&cm, "x", 0.0),
                y: f(&cm, "y", 0.0),
                w: f(&cm, "w", 220.0),
                h: f(&cm, "h", 80.0),
                color: s(&cm, "color", "#2b2c30"),
            });
        }
        if let Some(cl) = data.get("clusters").and_then(|x| x.as_object()) {
            g.clusters = cl.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        }
        g
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    }

    pub fn load(path: &std::path::Path) -> std::io::Result<Graph> {
        let text = std::fs::read_to_string(path)?;
        let data: J = serde_json::from_str(&text).map_err(std::io::Error::other)?;
        Ok(Graph::from_json(&data))
    }
}
