//! Quantities: a number with a unit, possibly a range.
//!
//! A unit is a vector of exponents over three bases — volts, amps and seconds — which is enough
//! for everything a board's facts are stated in: Ω is V/A, W is V·A, F is A·s/V, H is V·s/A, Hz
//! is 1/s and S is A/V. Arithmetic composes the exponents; a sum or a comparison of two different
//! units is an error, which is the whole reason to carry them.
//!
//! A quantity is always a range. A stated value is a range of zero width; a rail's `"3.3V"
//! tolerance="2%"` is one of 4 %; and arithmetic is interval arithmetic, so a lamp's current off a
//! 12–18 V rail is a range too, and `.min`/`.max` read its ends.

use std::fmt;

/// Exponents of volts, amps and seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Unit {
    pub v: i8,
    pub a: i8,
    pub s: i8,
}

// The arithmetic is checked (`add` and `div` can fail) and interval-valued, so it is a method set
// of its own rather than the `std::ops` traits, whose signatures cannot say so.
#[allow(clippy::should_implement_trait)]
impl Unit {
    pub const NONE: Unit = Unit { v: 0, a: 0, s: 0 };
    pub const VOLT: Unit = Unit { v: 1, a: 0, s: 0 };
    pub const AMP: Unit = Unit { v: 0, a: 1, s: 0 };
    pub const OHM: Unit = Unit { v: 1, a: -1, s: 0 };
    pub const WATT: Unit = Unit { v: 1, a: 1, s: 0 };
    pub const FARAD: Unit = Unit { v: -1, a: 1, s: 1 };
    pub const HENRY: Unit = Unit { v: 1, a: -1, s: 1 };
    pub const HERTZ: Unit = Unit { v: 0, a: 0, s: -1 };
    pub const SIEMENS: Unit = Unit { v: -1, a: 1, s: 0 };
    pub const SECOND: Unit = Unit { v: 0, a: 0, s: 1 };

    const SYMBOLS: [(&'static str, Unit); 9] = [
        ("V", Unit::VOLT),
        ("A", Unit::AMP),
        ("Ω", Unit::OHM),
        ("W", Unit::WATT),
        ("F", Unit::FARAD),
        ("H", Unit::HENRY),
        ("Hz", Unit::HERTZ),
        ("S", Unit::SIEMENS),
        ("s", Unit::SECOND),
    ];

    pub fn mul(self, o: Unit) -> Unit {
        Unit {
            v: self.v + o.v,
            a: self.a + o.a,
            s: self.s + o.s,
        }
    }

    pub fn div(self, o: Unit) -> Unit {
        Unit {
            v: self.v - o.v,
            a: self.a - o.a,
            s: self.s - o.s,
        }
    }

    /// The symbol, where the unit has one; otherwise the exponents spelled out.
    pub fn symbol(self) -> String {
        if self == Unit::NONE {
            return String::new();
        }
        if let Some((s, _)) = Unit::SYMBOLS.iter().find(|(_, u)| *u == self) {
            return s.to_string();
        }
        let mut out = String::new();
        for (name, e) in [("V", self.v), ("A", self.a), ("s", self.s)] {
            match e {
                0 => {}
                1 => out.push_str(name),
                e => out.push_str(&format!("{name}^{e}")),
            }
        }
        out
    }

    fn from_symbol(s: &str) -> Option<Unit> {
        match s {
            "ohm" | "R" => Some(Unit::OHM),
            _ => Unit::SYMBOLS.iter().find(|(n, _)| *n == s).map(|(_, u)| *u),
        }
    }
}

/// A closed interval with a unit. `lo == hi` is a plain value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    pub lo: f64,
    pub hi: f64,
    pub unit: Unit,
}

#[allow(clippy::should_implement_trait)]
impl Quantity {
    pub fn point(x: f64, unit: Unit) -> Quantity {
        Quantity { lo: x, hi: x, unit }
    }

    pub fn range(lo: f64, hi: f64, unit: Unit) -> Quantity {
        Quantity {
            lo: lo.min(hi),
            hi: lo.max(hi),
            unit,
        }
    }

    pub fn plain(x: f64) -> Quantity {
        Quantity::point(x, Unit::NONE)
    }

    pub fn is_point(&self) -> bool {
        self.lo == self.hi || (self.hi - self.lo).abs() <= 1e-12 * self.hi.abs().max(1e-300)
    }

    /// The value, for a point; the midpoint otherwise.
    pub fn mid(&self) -> f64 {
        (self.lo + self.hi) / 2.0
    }

    pub fn min(&self) -> Quantity {
        Quantity::point(self.lo, self.unit)
    }

    pub fn max(&self) -> Quantity {
        Quantity::point(self.hi, self.unit)
    }

    /// Widens a point by ±`fraction` (a plain ratio: 0.02 for 2 %).
    pub fn with_tolerance(self, fraction: f64) -> Quantity {
        let f = fraction.abs();
        Quantity::range(
            self.lo - self.lo.abs() * f,
            self.hi + self.hi.abs() * f,
            self.unit,
        )
    }

    pub fn add(self, o: Quantity) -> Result<Quantity, String> {
        if self.unit != o.unit {
            return Err(units_differ("add", self, o));
        }
        Ok(Quantity::range(self.lo + o.lo, self.hi + o.hi, self.unit))
    }

    pub fn sub(self, o: Quantity) -> Result<Quantity, String> {
        if self.unit != o.unit {
            return Err(units_differ("subtract", self, o));
        }
        Ok(Quantity::range(self.lo - o.hi, self.hi - o.lo, self.unit))
    }

    pub fn mul(self, o: Quantity) -> Quantity {
        let p = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        Quantity::range(
            p.iter().cloned().fold(f64::INFINITY, f64::min),
            p.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            self.unit.mul(o.unit),
        )
    }

    pub fn div(self, o: Quantity) -> Result<Quantity, String> {
        if o.lo <= 0.0 && o.hi >= 0.0 {
            return Err(format!("division by {o}, which spans zero"));
        }
        let p = [
            self.lo / o.lo,
            self.lo / o.hi,
            self.hi / o.lo,
            self.hi / o.hi,
        ];
        Ok(Quantity::range(
            p.iter().cloned().fold(f64::INFINITY, f64::min),
            p.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            self.unit.div(o.unit),
        ))
    }

    pub fn neg(self) -> Quantity {
        Quantity::range(-self.hi, -self.lo, self.unit)
    }

    /// Whether two quantities are the same value, to a rounding error.
    pub fn same(&self, o: &Quantity) -> bool {
        self.unit == o.unit && close(self.lo, o.lo) && close(self.hi, o.hi)
    }

    /// Parses `"4.7kΩ"`, `"100nF"`, `"3.3V"`, `"85%"`, `"0x48"`, `"0.5"`.
    pub fn parse(text: &str) -> Result<Quantity, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("an empty quantity".into());
        }
        if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return i64::from_str_radix(hex, 16)
                .map(|i| Quantity::plain(i as f64))
                .map_err(|_| format!("`{t}` is not a number"));
        }
        let split = t
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
            .map(|(i, _)| i)
            .unwrap_or(t.len());
        // `e` is a digit only when followed by one: `1e-6`, but not `1e` … there is no unit `e`.
        let (num, _) = t.split_at(split);
        let num = num.trim_end_matches(['e', 'E']);
        let rest = &t[num.len()..];
        let n: f64 = num
            .parse()
            .map_err(|_| format!("`{t}` is not a number with a unit"))?;
        let rest = rest.trim();
        if rest.is_empty() {
            return Ok(Quantity::plain(n));
        }
        if rest == "%" {
            return Ok(Quantity::plain(n / 100.0));
        }
        // The unit is the longest symbol the text ends with; whatever is left is the prefix.
        let mut candidates: Vec<&str> =
            vec!["Hz", "ohm", "V", "A", "Ω", "W", "F", "H", "S", "s", "R"];
        candidates.sort_by_key(|c| std::cmp::Reverse(c.len()));
        for sym in candidates {
            if let Some(prefix) = rest.strip_suffix(sym) {
                let scale = match prefix {
                    "" => 1.0,
                    "p" => 1e-12,
                    "n" => 1e-9,
                    "µ" | "u" => 1e-6,
                    "m" => 1e-3,
                    "k" => 1e3,
                    "M" => 1e6,
                    "G" => 1e9,
                    _ => continue,
                };
                let unit = Unit::from_symbol(sym).unwrap();
                return Ok(Quantity::point(n * scale, unit));
            }
        }
        Err(format!("`{t}` has no unit stackup knows: `{rest}`"))
    }

    /// Snaps to the nearest member of an E-series, in log space.
    pub fn e_series(self, series: &[f64]) -> Quantity {
        let snap = |x: f64| -> f64 {
            if x <= 0.0 || !x.is_finite() {
                return x;
            }
            let decade = x.log10().floor();
            let mut best = x;
            let mut best_d = f64::INFINITY;
            for d in [decade - 1.0, decade, decade + 1.0] {
                for m in series {
                    let cand = m * 10f64.powf(d);
                    let dist = (cand.ln() - x.ln()).abs();
                    if dist < best_d {
                        best_d = dist;
                        best = cand;
                    }
                }
            }
            best
        };
        Quantity::range(snap(self.lo), snap(self.hi), self.unit)
    }
}

pub const E3: [f64; 3] = [1.0, 2.2, 4.7];
pub const E6: [f64; 6] = [1.0, 1.5, 2.2, 3.3, 4.7, 6.8];
pub const E12: [f64; 12] = [1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2];
pub const E24: [f64; 24] = [
    1.0, 1.1, 1.2, 1.3, 1.5, 1.6, 1.8, 2.0, 2.2, 2.4, 2.7, 3.0, 3.3, 3.6, 3.9, 4.3, 4.7, 5.1, 5.6,
    6.2, 6.8, 7.5, 8.2, 9.1,
];
pub const E96: [f64; 96] = [
    1.00, 1.02, 1.05, 1.07, 1.10, 1.13, 1.15, 1.18, 1.21, 1.24, 1.27, 1.30, 1.33, 1.37, 1.40, 1.43,
    1.47, 1.50, 1.54, 1.58, 1.62, 1.65, 1.69, 1.74, 1.78, 1.82, 1.87, 1.91, 1.96, 2.00, 2.05, 2.10,
    2.15, 2.21, 2.26, 2.32, 2.37, 2.43, 2.49, 2.55, 2.61, 2.67, 2.74, 2.80, 2.87, 2.94, 3.01, 3.09,
    3.16, 3.24, 3.32, 3.40, 3.48, 3.57, 3.65, 3.74, 3.83, 3.92, 4.02, 4.12, 4.22, 4.32, 4.42, 4.53,
    4.64, 4.75, 4.87, 4.99, 5.11, 5.23, 5.36, 5.49, 5.62, 5.76, 5.90, 6.04, 6.19, 6.34, 6.49, 6.65,
    6.81, 6.98, 7.15, 7.32, 7.50, 7.68, 7.87, 8.06, 8.25, 8.45, 8.66, 8.87, 9.09, 9.31, 9.53, 9.76,
];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-300)
}

fn units_differ(verb: &str, a: Quantity, b: Quantity) -> String {
    format!(
        "cannot {verb} {} and {}: the units differ",
        describe_unit(a.unit),
        describe_unit(b.unit)
    )
}

fn describe_unit(u: Unit) -> String {
    let s = u.symbol();
    if s.is_empty() {
        "a plain number".into()
    } else {
        format!("`{s}`")
    }
}

/// One number in engineering notation: three significant figures, an SI prefix, the unit.
pub fn engineering(x: f64, unit: Unit) -> String {
    if x == 0.0 {
        return format!("0{}", unit.symbol());
    }
    if !x.is_finite() {
        return format!("{x}{}", unit.symbol());
    }
    let prefixes: [(f64, &str); 8] = [
        (1e9, "G"),
        (1e6, "M"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "µ"),
        (1e-9, "n"),
        (1e-12, "p"),
    ];
    let mag = x.abs();
    let (scale, prefix) = if unit == Unit::NONE {
        (1.0, "")
    } else {
        prefixes
            .iter()
            .find(|(s, _)| mag >= *s * 0.9995)
            .cloned()
            .unwrap_or((1e-12, "p"))
    };
    let m = x / scale;
    // Three significant figures, trailing zeros dropped.
    let digits = 3 - (m.abs().log10().floor() as i32 + 1).clamp(-6, 3);
    let s = format!("{m:.*}", digits.max(0) as usize);
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    format!("{s}{prefix}{}", unit.symbol())
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_point() {
            f.write_str(&engineering(self.lo, self.unit))
        } else {
            write!(
                f,
                "{}–{}",
                engineering(self.lo, self.unit),
                engineering(self.hi, self.unit)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(
            Quantity::parse("4.7kΩ").unwrap(),
            Quantity::point(4700.0, Unit::OHM)
        );
        assert!(
            Quantity::parse("100nF")
                .unwrap()
                .same(&Quantity::point(100e-9, Unit::FARAD))
        );
        assert_eq!(
            Quantity::parse("500mA").unwrap(),
            Quantity::point(0.5, Unit::AMP)
        );
        assert_eq!(
            Quantity::parse("2Hz").unwrap(),
            Quantity::point(2.0, Unit::HERTZ)
        );
        assert_eq!(
            Quantity::parse("10ms").unwrap(),
            Quantity::point(0.01, Unit::SECOND)
        );
        assert_eq!(
            Quantity::parse("1MHz").unwrap(),
            Quantity::point(1e6, Unit::HERTZ)
        );
        assert_eq!(Quantity::parse("85%").unwrap(), Quantity::plain(0.85));
        assert_eq!(Quantity::parse("0x48").unwrap(), Quantity::plain(72.0));
        assert_eq!(Quantity::parse("0.5").unwrap(), Quantity::plain(0.5));
        assert!(
            Quantity::parse("10uH")
                .unwrap()
                .same(&Quantity::point(10e-6, Unit::HENRY))
        );
        assert!(Quantity::parse("3.3X").is_err());
        assert!(Quantity::parse("abc").is_err());
    }

    #[test]
    fn arithmetic_and_units() {
        let v = Quantity::parse("3.3V").unwrap();
        let r = Quantity::parse("330Ω").unwrap();
        let i = v.div(r).unwrap();
        assert_eq!(i.unit, Unit::AMP);
        assert_eq!(i.to_string(), "10mA");
        assert_eq!(v.mul(i).to_string(), "33mW");
        assert_eq!(Quantity::plain(1.0).div(r).unwrap().unit, Unit::SIEMENS);
        assert!(v.add(i).is_err());
        let rc = r.mul(Quantity::parse("100nF").unwrap());
        assert_eq!(rc.unit, Unit::SECOND);
        assert_eq!(rc.to_string(), "33µs");
    }

    #[test]
    fn ranges() {
        let rail = Quantity::parse("3.3V").unwrap().with_tolerance(0.02);
        assert_eq!(rail.to_string(), "3.23V–3.37V");
        let fwd = Quantity::parse("2V").unwrap();
        let across = rail.sub(fwd).unwrap();
        assert_eq!(across.min().to_string(), "1.23V");
        assert_eq!(across.max().to_string(), "1.37V");
        let wide = Quantity::range(12.0, 18.0, Unit::VOLT);
        let ratio = Quantity::plain(0.1525);
        assert_eq!(wide.mul(ratio).to_string(), "1.83V–2.75V");
    }

    #[test]
    fn formatting() {
        assert_eq!(engineering(4700.0, Unit::OHM), "4.7kΩ");
        assert_eq!(engineering(0.00062, Unit::AMP), "620µA");
        assert_eq!(engineering(1e-6, Unit::FARAD), "1µF");
        assert_eq!(engineering(0.5, Unit::NONE), "0.5");
        assert_eq!(engineering(72.0, Unit::NONE), "72");
        assert_eq!(engineering(999.6, Unit::OHM), "1kΩ");
        assert_eq!(engineering(0.0, Unit::AMP), "0A");
    }

    #[test]
    fn e_series() {
        let x = Quantity::point(4300.0, Unit::OHM);
        assert_eq!(x.e_series(&E24).to_string(), "4.3kΩ");
        assert_eq!(
            Quantity::point(4400.0, Unit::OHM)
                .e_series(&E24)
                .to_string(),
            "4.3kΩ"
        );
        assert_eq!(
            Quantity::point(3e-9, Unit::FARAD).e_series(&E3).to_string(),
            "2.2nF"
        );
        assert_eq!(
            Quantity::point(0.9, Unit::NONE).e_series(&E3).to_string(),
            "1"
        );
    }
}
