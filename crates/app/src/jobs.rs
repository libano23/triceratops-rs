//! Long operations run as background jobs: the window keeps drawing, the status bar shows
//! what is running, and Esc cancels it.
//!
//! A command starts a job with `CmdCtx::work`: the computation runs on a worker thread with a
//! `tri_geom::job::Token` installed, and its result is applied to the document on the GUI
//! thread when the job ends (`App::poll_job`, called every frame).

use std::time::Duration;

/// What the status bar and `get_state` show about the running job.
#[derive(Clone, Debug, PartialEq)]
pub struct JobStatus {
    pub label: String,
    pub elapsed: Duration,
    pub stage: &'static str,
    pub count: u64,
    /// Esc was pressed (or `cancel` called) and the job is winding down.
    pub cancelling: bool,
}

impl JobStatus {
    /// `BooleanUnion — 12 s · mesh Boolean · 1.3 M · Esc to cancel` (the status bar draws a
    /// spinner in front; the UI font has no arrow glyph)
    pub fn text(&self) -> String {
        let mut s = format!("{} — {} s", self.label, self.elapsed.as_secs());
        if !self.stage.is_empty() {
            s.push_str(&format!(" · {}", self.stage));
        }
        if self.count > 0 {
            s.push_str(&format!(" · {}", count_text(self.count)));
        }
        s.push_str(if self.cancelling { " · cancelling…" } else { " · Esc to cancel" });
        s
    }
}

/// 950, 12.3 k, 1.3 M
pub fn count_text(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1} k", n as f64 / 1e3),
        _ => format!("{:.1} M", n as f64 / 1e6),
    }
}

/// Test helpers shared by the job, handler and host tests.
#[cfg(test)]
pub mod testing {
    use glam::DVec3;
    use tri_geom::Mesh;

    /// A closed UV sphere of about `2 * seg * seg` triangles (seg 100: a Boolean of two
    /// overlapping ones takes long enough to watch and cancel).
    pub fn uv_sphere(c: DVec3, r: f64, seg: usize) -> Mesh {
        let mut m = Mesh::new();
        for i in 0..=seg {
            let th = std::f64::consts::PI * i as f64 / seg as f64;
            for j in 0..seg {
                let ph = std::f64::consts::TAU * j as f64 / seg as f64;
                m.vertices.push(c + r * DVec3::new(th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()));
            }
        }
        let idx = |i: usize, j: usize| (i * seg + j % seg) as u32;
        for i in 0..seg {
            for j in 0..seg {
                let (a, b, cc, d) = (idx(i, j), idx(i, j + 1), idx(i + 1, j + 1), idx(i + 1, j));
                if i != 0 {
                    m.indices.extend_from_slice(&[a, d, b]);
                }
                if i != seg - 1 {
                    m.indices.extend_from_slice(&[b, d, cc]);
                }
            }
        }
        m.compute_normals();
        m
    }


    /// Two overlapping 20k-triangle mesh spheres, #1 and #2 in a fresh document.
    pub fn heavy_spheres(app: &mut crate::app::App) {
        app.doc.add(tri_doc::Geometry::Mesh(uv_sphere(DVec3::ZERO, 10.0, 100)));
        app.doc.add(tri_doc::Geometry::Mesh(uv_sphere(DVec3::new(5.0, 0.0, 0.0), 10.0, 100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use std::time::Instant;

    /// Poll like the frame loop does until the job is over (or panic after `limit`).
    fn wait(app: &mut App, limit: Duration) {
        let t = Instant::now();
        while app.job_status().is_some() {
            app.poll_job();
            assert!(t.elapsed() < limit, "job still running after {limit:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
        app.poll_job();
    }

    fn run(app: &mut App, lines: &[&str]) {
        app.run_script_lines(&lines.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }

    #[test]
    fn status_text_reads_well() {
        let s = JobStatus { label: "BooleanUnion".into(), elapsed: Duration::from_secs(12), stage: "mesh Boolean", count: 1_300_000, cancelling: false };
        assert_eq!(s.text(), "BooleanUnion — 12 s · mesh Boolean · 1.3 M · Esc to cancel");
        assert_eq!(count_text(950), "950");
        assert_eq!(count_text(12_345), "12.3 k");
    }

    #[test]
    fn boolean_runs_as_job() {
        let mut app = App::new();
        run(&mut app, &["Box", "0,0,0", "10,10,0", "10", "Box", "5,5,5", "15,15,5", "10"]);
        run(&mut app, &["BooleanUnion", "#1,#2", ""]);
        assert_eq!(app.job_status().map(|s| s.label), Some("BooleanUnion".to_string()), "log: {:?}", app.log);
        wait(&mut app, Duration::from_secs(60));
        assert_eq!(app.doc.objects().count(), 1, "log: {:?}", app.log);
        assert!(app.job_status().is_none());
        assert!(!app.runner.is_active());
    }

    #[test]
    fn esc_cancels_boolean_job() {
        let mut app = App::new();
        super::testing::heavy_spheres(&mut app);
        run(&mut app, &["BooleanUnion", "#1,#2", ""]);
        assert!(app.job_status().is_some(), "log: {:?}", app.log);
        std::thread::sleep(Duration::from_millis(50));
        app.cancel(); // what Esc does
        let t = Instant::now();
        wait(&mut app, Duration::from_secs(5));
        assert!(t.elapsed() < Duration::from_millis(1500), "took {:?}", t.elapsed());
        assert_eq!(app.doc.objects().count(), 2, "document unchanged");
        assert!(app.log.iter().any(|l| l.contains("BooleanUnion cancelled by the user")), "log: {:?}", app.log);
        assert!(!app.runner.is_active());
    }

    #[test]
    fn commands_refused_while_job_runs() {
        let mut app = App::new();
        super::testing::heavy_spheres(&mut app);
        app.assistant_mode = false;
        app.cmd_text = "BooleanUnion".into();
        app.submit();
        app.cmd_text = "#1,#2".into();
        app.submit();
        app.submit_enter();
        assert!(app.job_status().is_some(), "log: {:?}", app.log);
        app.cmd_text = "Box".into();
        app.submit();
        assert!(app.log.last().is_some_and(|l| l.contains("BooleanUnion is running")), "log: {:?}", app.log);
        app.cancel();
        wait(&mut app, Duration::from_secs(5));
    }

    #[test]
    fn script_lines_wait_for_job() {
        let mut app = App::new();
        run(&mut app, &["Box", "0,0,0", "10,10,0", "10", "Box", "5,5,5", "15,15,5", "10", "BooleanUnion", "#1,#2", "", "Sphere", "50,0,0", "5"]);
        assert!(app.job_status().is_some());
        assert_eq!(app.doc.objects().count(), 2, "the sphere waits for the union");
        wait(&mut app, Duration::from_secs(60));
        assert_eq!(app.doc.objects().count(), 2, "union + sphere; log: {:?}", app.log);
        assert!(app.pending_lines.is_empty());
    }

    /// Every command that runs a solid Boolean does it as a job.
    #[test]
    fn boolean_commands_run_as_jobs() {
        let boxes = ["Box", "0,0,0", "10,10,0", "10", "Box", "5,5,5", "15,15,5", "10"];
        let cases: &[(&str, &[&str])] = &[
            ("BooleanSplit", &["BooleanSplit", "#1", "#2"]),
            ("Boolean2Objects", &["Boolean2Objects", "#1", "#2", "Difference"]),
            ("MeshBooleanUnion", &["MeshBooleanUnion", "#1,#2", ""]),
            ("MeshBooleanDifference", &["MeshBooleanDifference", "#1", "#2"]),
            ("MeshBooleanSplit", &["MeshBooleanSplit", "#1", "#2"]),
        ];
        for (label, lines) in cases {
            let mut app = App::new();
            run(&mut app, &boxes);
            run(&mut app, lines);
            assert_eq!(app.job_status().map(|s| s.label), Some(label.to_string()), "{label}: log {:?}", app.log);
            wait(&mut app, Duration::from_secs(60));
            assert!(app.doc.objects().count() >= 1, "{label}: log {:?}", app.log);
            assert!(!app.runner.is_active(), "{label}");
        }
    }

    /// Review: the user starting a command while an assistant's job runs must not take the
    /// job away from the assistant.
    #[test]
    fn user_command_during_assistant_job_keeps_ownership() {
        let mut app = App::new();
        super::testing::heavy_spheres(&mut app);
        app.assistant_mode = true;
        run(&mut app, &["BooleanUnion", "#1,#2", ""]);
        app.assistant_mode = false;
        assert!(app.job_status().is_some());
        app.start_command("Undo");
        assert!(app.assistant_owns_command, "still the assistant's job");
        assert!(app.log.last().is_some_and(|l| l.contains("is running")), "{:?}", app.log.last());
        app.cancel_job("cancelled");
        wait(&mut app, Duration::from_secs(5));
    }

    /// Script lines an assistant queued behind a job are dropped, not typed over what the
    /// user is typing when the job ends.
    #[test]
    fn pending_assistant_lines_yield_to_typing() {
        let mut app = App::new();
        app.assistant_mode = true;
        run(&mut app, &["Box", "0,0,0", "10,10,0", "10", "Box", "5,5,5", "15,15,5", "10", "BooleanUnion", "#1,#2", "", "Sphere", "50,0,0", "5"]);
        app.assistant_mode = false;
        assert!(!app.pending_lines.is_empty());
        app.cmd_text = "Lin".into();
        wait(&mut app, Duration::from_secs(60));
        assert_eq!(app.cmd_text, "Lin", "the user's text is kept");
        assert_eq!(app.doc.objects().count(), 1, "the sphere was not made");
        assert!(app.log.iter().any(|l| l.contains("abandoned")), "{:?}", app.log);
        assert!(app.pending_lines.is_empty());
    }

    /// Esc leaves the user's own command before it stops a Locust solve.
    #[test]
    fn esc_leaves_the_users_command_first() {
        let mut app = App::new();
        let g = crate::locust::authoring::prepare(&serde_json::json!({
            "nodes": [{ "type": "Populate 2D", "nickname": "P", "values": { "Count": 5000 } }, { "type": "Voronoi" }],
            "connections": [{ "from": "P", "to": "Voronoi.Points" }]
        }))
        .unwrap();
        app.locust.replace_root(g);
        app.locust.solve(&app.doc);
        app.cmd_text = "Circle".into();
        app.submit();
        assert!(app.runner.is_active() && app.locust.solving().is_some());
        app.cancel();
        assert!(!app.runner.is_active(), "the command went first");
        assert!(app.locust.solving().is_some(), "the solve goes on");
        app.cancel();
        let t = Instant::now();
        while app.locust.solving().is_some() {
            app.locust.poll();
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
