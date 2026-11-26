//! Command-line input parsing: coordinates, relative/world prefixes, polar input,
//! distance and angle constraints and the command-line calculator.

use glam::DVec3;
use tri_geom::Plane;

/// Parsed value typed at a point prompt.
#[derive(Clone, Debug, PartialEq)]
pub enum PointInput {
    /// A fully specified point.
    Point(DVec3),
    /// Distance constraint: the marker moves around the base point at this distance.
    /// Negative values mean incremental distances.
    Distance(f64),
    /// Angle constraint in degrees, measured in the construction plane from the base point.
    Angle(f64),
    /// Both distance and angle (polar) relative to the base point.
    Polar { distance: f64, angle_deg: f64 },
    /// A relative MicroStation key-in (`DL=`, `DX=`, `DI=`) typed with no previous point.
    NeedsBase(&'static str),
}

/// Evaluate a small arithmetic expression: numbers, + - * / ^, parentheses,
/// sin cos tan asin acos atan sqrt abs, pi, e. Trig functions use degrees.
pub fn calc(expr: &str) -> Option<f64> {
    let s: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return None;
    }
    let mut p = Parser { s: &s, i: 0 };
    let v = p.expr()?;
    if p.i == s.len() && v.is_finite() {
        Some(v)
    } else {
        None
    }
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                '+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                '-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c) = self.peek() {
            match c {
                '*' => {
                    self.i += 1;
                    v *= self.power()?;
                }
                '/' => {
                    self.i += 1;
                    let d = self.power()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.peek() == Some('^') {
            self.i += 1;
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some('-') => {
                self.i += 1;
                Some(-self.unary()?)
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if c == '(' {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() != Some(')') {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        if c.is_ascii_digit() || c == '.' {
            let start = self.i;
            while let Some(d) = self.peek() {
                if d.is_ascii_digit() || d == '.' || d == 'e' || d == 'E' {
                    // allow exponent sign
                    self.i += 1;
                    if (d == 'e' || d == 'E') && matches!(self.peek(), Some('+') | Some('-')) {
                        self.i += 1;
                    }
                } else {
                    break;
                }
            }
            let txt: String = self.s[start..self.i].iter().collect();
            return txt.parse().ok();
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while matches!(self.peek(), Some(d) if d.is_ascii_alphanumeric()) {
                self.i += 1;
            }
            let name: String = self.s[start..self.i].iter().collect::<String>().to_ascii_lowercase();
            match name.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            if self.peek() != Some('(') {
                return None;
            }
            self.i += 1;
            let a = self.expr()?;
            if self.peek() != Some(')') {
                return None;
            }
            self.i += 1;
            return Some(match name.as_str() {
                "sin" => a.to_radians().sin(),
                "cos" => a.to_radians().cos(),
                "tan" => a.to_radians().tan(),
                "asin" => a.asin().to_degrees(),
                "acos" => a.acos().to_degrees(),
                "atan" => a.atan().to_degrees(),
                "sqrt" => a.sqrt(),
                "abs" => a.abs(),
                "ln" => a.ln(),
                "log" => a.log10(),
                "exp" => a.exp(),
                _ => return None,
            });
        }
        None
    }
}

/// Parse a number (allowing calculator expressions).
pub fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().or_else(|| calc(t))
}

/// Parse text typed at a point prompt following Triceratops' conventions.
///
/// * `x,y` / `x,y,z`: construction plane coordinates
/// * `w` prefix: world, `r` or `@` prefix: relative to `base`, `wr`: world relative
/// * `d<a` polar (optionally `d<a,z`), `@d<a` relative polar
/// * a single number: distance constraint; `<a`: angle constraint
/// * MicroStation key-ins: `XY=x,y[,z]` absolute world, `DL=dx,dy[,dz]` world offset from
///   `base`, `DX=dx,dy[,dz]` offset along the construction plane's axes, `DI=d,a` polar
///   from `base` in the construction plane
pub fn parse_point_input(text: &str, cplane: &Plane, base: Option<DVec3>) -> Option<PointInput> {
    let mut t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Some(r) = microstation_keyin(&t, cplane, base) {
        return r;
    }
    let mut world = false;
    let mut relative = false;
    loop {
        if let Some(rest) = t.strip_prefix("wr") {
            world = true;
            relative = true;
            t = rest.to_string();
        } else if let Some(rest) = t.strip_prefix('w') {
            world = true;
            t = rest.to_string();
        } else if let Some(rest) = t.strip_prefix('r').or_else(|| t.strip_prefix('@')) {
            relative = true;
            t = rest.to_string();
        } else {
            break;
        }
    }
    // angle constraint "<45"
    if let Some(rest) = t.strip_prefix('<') {
        if !rest.contains(',') {
            return parse_number(rest).map(PointInput::Angle);
        }
    }
    // polar "d<a" or "d<a,z"
    if let Some((d, rest)) = t.split_once('<') {
        let (a, z) = match rest.split_once(',') {
            Some((a, z)) => (a, Some(z)),
            None => (rest, None),
        };
        let d = parse_number(d)?;
        let a = parse_number(a)?;
        let z = match z {
            Some(z) => Some(parse_number(z)?),
            None => None,
        };
        let origin = if relative { base? } else if world { DVec3::ZERO } else { cplane.origin };
        let (ax, ay, az) = if world { (DVec3::X, DVec3::Y, DVec3::Z) } else { (cplane.x_axis, cplane.y_axis, cplane.z_axis) };
        let (sa, ca) = a.to_radians().sin_cos();
        let p = origin + ax * (d * ca) + ay * (d * sa) + az * z.unwrap_or(0.0);
        if relative && z.is_none() && !world {
            return Some(PointInput::Polar { distance: d, angle_deg: a });
        }
        return Some(PointInput::Point(p));
    }
    let parts: Vec<&str> = t.split(',').collect();
    match parts.len() {
        1 => {
            if relative || world {
                return None;
            }
            parse_number(parts[0]).map(PointInput::Distance)
        }
        2 | 3 => {
            let x = parse_number(parts[0])?;
            let y = parse_number(parts[1])?;
            let z = if parts.len() == 3 { parse_number(parts[2])? } else { 0.0 };
            let v = if world { DVec3::new(x, y, z) } else { cplane.x_axis * x + cplane.y_axis * y + cplane.z_axis * z };
            let p = if relative { base? + v } else if world { v } else { cplane.origin + v };
            Some(PointInput::Point(p))
        }
        _ => None,
    }
}

/// Whether `text` has the shape of a MicroStation key-in (`XY=`, `DL=`, `DX=`, `DI=`), so
/// that at a point prompt it is read as one even when an option name starts the same way.
pub fn is_keyin(text: &str) -> bool {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    matches!(t.split_once('='), Some(("xy" | "dl" | "dx" | "di", _)))
}

/// A MicroStation precision key-in: `None` when the text is not one, `Some(None)` when
/// it is one but malformed, `Some(Some(_))` otherwise.
fn microstation_keyin(text: &str, cplane: &Plane, base: Option<DVec3>) -> Option<Option<PointInput>> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let (key, rest) = t.split_once('=')?;
    let name = match key {
        "xy" => "XY=",
        "dl" => "DL=",
        "dx" => "DX=",
        "di" => "DI=",
        _ => return None,
    };
    let nums: Option<Vec<f64>> = rest.split(',').map(parse_number).collect();
    let Some(n) = nums else { return Some(None) };
    let ok = if name == "DI=" { n.len() == 2 } else { n.len() == 2 || n.len() == 3 };
    if !ok {
        return Some(None);
    }
    let z = n.get(2).copied().unwrap_or(0.0);
    if name == "XY=" {
        return Some(Some(PointInput::Point(DVec3::new(n[0], n[1], z))));
    }
    let Some(b) = base else { return Some(Some(PointInput::NeedsBase(name))) };
    let p = match name {
        "DL=" => b + DVec3::new(n[0], n[1], z),
        "DX=" => b + cplane.x_axis * n[0] + cplane.y_axis * n[1] + cplane.z_axis * z,
        _ => {
            let (s, c) = n[1].to_radians().sin_cos();
            b + cplane.x_axis * (n[0] * c) + cplane.y_axis * (n[0] * s)
        }
    };
    Some(Some(PointInput::Point(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator() {
        assert_eq!(calc("1+2*3"), Some(7.0));
        assert_eq!(calc("(1+2)*3"), Some(9.0));
        assert_eq!(calc("2^3^2"), Some(512.0));
        assert!((calc("sin(30)").unwrap() - 0.5).abs() < 1e-12);
        assert!((calc("sqrt(16)/2").unwrap() - 2.0).abs() < 1e-12);
        assert_eq!(calc("1/0"), None);
        assert_eq!(calc("abc"), None);
    }

    #[test]
    fn coordinates() {
        let cp = Plane::world_xz();
        assert_eq!(parse_point_input("1,2", &cp, None), Some(PointInput::Point(DVec3::new(1.0, 0.0, 2.0))));
        assert_eq!(parse_point_input("w1,2,3", &cp, None), Some(PointInput::Point(DVec3::new(1.0, 2.0, 3.0))));
        let b = Some(DVec3::new(5.0, 5.0, 5.0));
        assert_eq!(parse_point_input("r1,0", &cp, b), Some(PointInput::Point(DVec3::new(6.0, 5.0, 5.0))));
        assert_eq!(parse_point_input("@0,1", &cp, b), Some(PointInput::Point(DVec3::new(5.0, 5.0, 6.0))));
        assert_eq!(parse_point_input("wr0,1", &cp, b), Some(PointInput::Point(DVec3::new(5.0, 6.0, 5.0))));
        assert_eq!(parse_point_input("5", &cp, b), Some(PointInput::Distance(5.0)));
        assert_eq!(parse_point_input("<45", &cp, b), Some(PointInput::Angle(45.0)));
        assert_eq!(parse_point_input("@4<90", &cp, b), Some(PointInput::Polar { distance: 4.0, angle_deg: 90.0 }));
        let p = parse_point_input("4<90", &Plane::world_xy(), None).unwrap();
        if let PointInput::Point(p) = p {
            assert!((p - DVec3::new(0.0, 4.0, 0.0)).length() < 1e-12);
        } else {
            panic!();
        }
        assert_eq!(parse_point_input("x", &cp, None), None);
    }

    #[test]
    fn microstation_keyins() {
        let cp = Plane::world_xz();
        let o = DVec3::new(5.0, 5.0, 5.0);
        let b = Some(o);
        let near = |got: Option<PointInput>, want: DVec3| match got {
            Some(PointInput::Point(p)) => assert!((p - want).length() < 1e-12, "{p} != {want}"),
            other => panic!("{other:?} != {want}"),
        };
        near(parse_point_input("XY=1,2,3", &cp, None), DVec3::new(1.0, 2.0, 3.0));
        near(parse_point_input("xy = 1 , 2", &cp, None), DVec3::new(1.0, 2.0, 0.0));
        near(parse_point_input("dl=1,2,3", &cp, b), DVec3::new(6.0, 7.0, 8.0));
        near(parse_point_input("DX=1,0", &cp, b), o + cp.x_axis);
        near(parse_point_input("dx=0,1,2", &cp, b), o + cp.y_axis + cp.z_axis * 2.0);
        near(parse_point_input("di=2,90", &cp, b), o + cp.y_axis * 2.0);
        near(parse_point_input("Di=2,180", &cp, b), o - cp.x_axis * 2.0);
        near(parse_point_input("dl=10/2,0", &cp, b), DVec3::new(10.0, 5.0, 5.0));
        // a key-in at a prompt with a base is a point even when it would read as one number
        near(parse_point_input("xy=3,4", &cp, b), DVec3::new(3.0, 4.0, 0.0));
        for (k, name) in [("dl=1,2", "DL="), ("dx=1,2", "DX="), ("di=3,0", "DI=")] {
            assert_eq!(parse_point_input(k, &cp, None), Some(PointInput::NeedsBase(name)), "{k}");
        }
        for bad in ["dl=", "dl=1", "xy=1,2,3,4", "di=5", "di=1,2,3", "zz=1,2", "xy=a,b", "xy=1,,2"] {
            assert_eq!(parse_point_input(bad, &cp, b), None, "{bad}");
        }
    }
}
