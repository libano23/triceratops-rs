//! B-spline basis function evaluation (NURBS Book algorithms A2.1 – A2.3).

/// Find the knot span index for parameter `u`.
/// `n` is the index of the last control point (count - 1), `p` the degree.
pub fn find_span(n: usize, p: usize, u: f64, knots: &[f64]) -> usize {
    if u >= knots[n + 1] {
        return n;
    }
    if u <= knots[p] {
        return p;
    }
    let mut low = p;
    let mut high = n + 1;
    let mut mid = (low + high) / 2;
    while u < knots[mid] || u >= knots[mid + 1] {
        if u < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }
        mid = (low + high) / 2;
    }
    mid
}

/// Compute the `p + 1` non-zero basis functions at `u` for span `i`.
pub fn basis_funs(i: usize, u: f64, p: usize, knots: &[f64]) -> Vec<f64> {
    let mut n = vec![0.0; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    n[0] = 1.0;
    for j in 1..=p {
        left[j] = u - knots[i + 1 - j];
        right[j] = knots[i + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            let denom = right[r + 1] + left[j - r];
            let temp = if denom.abs() < 1e-300 { 0.0 } else { n[r] / denom };
            n[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        n[j] = saved;
    }
    n
}

/// Compute basis functions and their derivatives up to order `nd`.
/// Returns `ders[k][j]` = k-th derivative of basis function `i - p + j`.
pub fn ders_basis_funs(i: usize, u: f64, p: usize, nd: usize, knots: &[f64]) -> Vec<Vec<f64>> {
    let nd = nd.min(p);
    let mut ndu = vec![vec![0.0; p + 1]; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    ndu[0][0] = 1.0;
    for j in 1..=p {
        left[j] = u - knots[i + 1 - j];
        right[j] = knots[i + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            ndu[j][r] = right[r + 1] + left[j - r];
            let temp = if ndu[j][r].abs() < 1e-300 { 0.0 } else { ndu[r][j - 1] / ndu[j][r] };
            ndu[r][j] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        ndu[j][j] = saved;
    }
    let mut ders = vec![vec![0.0; p + 1]; nd + 1];
    for j in 0..=p {
        ders[0][j] = ndu[j][p];
    }
    let mut a = vec![vec![0.0; p + 1]; 2];
    for r in 0..=p {
        let (mut s1, mut s2) = (0usize, 1usize);
        a[0][0] = 1.0;
        for k in 1..=nd {
            let mut d = 0.0;
            let rk = r as isize - k as isize;
            let pk = p as isize - k as isize;
            if r >= k {
                a[s2][0] = a[s1][0] / ndu[(pk + 1) as usize][rk as usize];
                d = a[s2][0] * ndu[rk as usize][pk as usize];
            }
            let j1 = if rk >= -1 { 1 } else { (-rk) as usize };
            let j2 = if (r as isize - 1) <= pk { k - 1 } else { p - r };
            for j in j1..=j2 {
                a[s2][j] = (a[s1][j] - a[s1][j - 1]) / ndu[(pk + 1) as usize][(rk + j as isize) as usize];
                d += a[s2][j] * ndu[(rk + j as isize) as usize][pk as usize];
            }
            if r <= pk as usize {
                a[s2][k] = -a[s1][k - 1] / ndu[(pk + 1) as usize][r];
                d += a[s2][k] * ndu[r][pk as usize];
            }
            ders[k][r] = d;
            std::mem::swap(&mut s1, &mut s2);
        }
    }
    let mut r = p as f64;
    for k in 1..=nd {
        for j in 0..=p {
            ders[k][j] *= r;
        }
        r *= (p - k) as f64;
    }
    ders
}

/// Build a clamped uniform knot vector for `n_cp` control points of degree `p`.
pub fn uniform_clamped_knots(n_cp: usize, p: usize) -> Vec<f64> {
    let m = n_cp + p + 1;
    let interior = n_cp - p - 1;
    let mut k = Vec::with_capacity(m);
    k.extend(std::iter::repeat_n(0.0, p + 1));
    k.extend((1..=interior).map(|i| i as f64 / (interior + 1) as f64));
    k.extend(std::iter::repeat_n(1.0, p + 1));
    k
}

/// Validate a knot vector against a control point count and degree.
pub fn validate_knots(n_cp: usize, p: usize, knots: &[f64]) -> crate::Result<()> {
    if n_cp < p + 1 {
        return Err(crate::GeomError::Invalid(format!(
            "degree {p} needs at least {} control points, got {n_cp}",
            p + 1
        )));
    }
    if knots.len() != n_cp + p + 1 {
        return Err(crate::GeomError::Invalid(format!(
            "knot vector length {} != {} (cp {n_cp} + degree {p} + 1)",
            knots.len(),
            n_cp + p + 1
        )));
    }
    if knots.windows(2).any(|w| w[1] < w[0]) {
        return Err(crate::GeomError::Invalid("knots must be non-decreasing".into()));
    }
    if knots[0] >= knots[knots.len() - 1] {
        return Err(crate::GeomError::Invalid("knot vector has zero span".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_of_unity() {
        let knots = uniform_clamped_knots(6, 3);
        for i in 0..=100 {
            let u = i as f64 / 100.0;
            let span = find_span(5, 3, u, &knots);
            let n = basis_funs(span, u, 3, &knots);
            assert!((n.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn derivative_of_sum_is_zero() {
        let knots = uniform_clamped_knots(7, 3);
        let u = 0.37;
        let span = find_span(6, 3, u, &knots);
        let d = ders_basis_funs(span, u, 3, 2, &knots);
        assert!((d[0].iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(d[1].iter().sum::<f64>().abs() < 1e-10);
    }
}
