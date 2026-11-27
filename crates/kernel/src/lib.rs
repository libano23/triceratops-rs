//! tri-kernel: exact B-rep solids on the solidcore kernel (feature `solidcore`).

#[cfg(all(test, feature = "solidcore"))]
mod tests {
    #[test]
    fn kernel_links_and_makes_a_box() {
        let body = solidcore_ops::body::make_box([0.0; 3], [2.0, 3.0, 4.0]).unwrap();
        let v = body.volume(1e-6).unwrap();
        assert!((v - 24.0).abs() < 1e-9, "volume {v}");
    }
}
