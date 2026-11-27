//! The exact body a solid may carry next to its faces (docs/superpowers/specs/2026-11-27-solidcore-kernel-design.md).
//!
//! `Brep::faces` stay authoritative for everything that reads faces. An exact body is a
//! solidcore B-rep record made together with those faces; it is trusted only while the
//! fingerprint of the faces still matches, so a command that edits faces in place can never
//! leave a stale exact body behind.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::brep::Face;

/// A solidcore record and the fingerprint of the faces it was made with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExactBody {
    pub record: Arc<solidcore_record::brep::Brep>,
    pub faces_fingerprint: u64,
}

/// FNV-1a, 64 bit. Stored in documents, so it is fixed here and does not depend on the
/// standard library's hasher.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
}

/// Hash of everything in `faces` that defines geometry: surfaces (degrees, counts, control
/// points, weights, knots) and trim loops, in order, plus `closed`.
pub fn fingerprint(faces: &[Face], closed: bool) -> u64 {
    let mut h = Fnv::new();
    h.u64(closed as u64);
    h.u64(faces.len() as u64);
    for f in faces {
        let s = &f.surface;
        for n in [s.degree_u, s.degree_v, s.count_u, s.count_v] {
            h.u64(n as u64);
        }
        for p in &s.control_points {
            h.f64(p.x);
            h.f64(p.y);
            h.f64(p.z);
        }
        for w in &s.weights {
            h.f64(*w);
        }
        for k in s.knots_u.iter().chain(&s.knots_v) {
            h.f64(*k);
        }
        h.u64(f.loops.len() as u64);
        for l in &f.loops {
            h.u64(l.len() as u64);
            for p in l {
                h.f64(p.x);
                h.f64(p.y);
            }
        }
    }
    h.0
}
