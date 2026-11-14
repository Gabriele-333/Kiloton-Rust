//! Numbers as the Java edition prints them. Java's `String.format("%.Nf")` does not round the exact
//! binary value: it takes the shortest decimal that reads back as the same double (what
//! `Double.toString` prints) and rounds that half up. So 12.35, which is 12.3499999... in binary,
//! prints "12.4" in Java and "12.3" with Rust's `{:.1}`. The text a player reads should not depend
//! on the build, so Kiloton's numbers go through [`fixed`].

/// `x` with `decimals` digits after the point, as Java's `%.Nf` prints it (`Locale.ROOT`).
pub fn fixed(x: f64, decimals: usize) -> String {
    if !x.is_finite() {
        return if x.is_nan() {
            "NaN".into()
        } else if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    // The shortest decimal of |x| in scientific form: "1.235e1" is 1.235 * 10^1.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp has an exponent");
    let exp: i64 = exp.parse().expect("an integer exponent");
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    // How many of those digits come before the cut, `decimals` places after the point.
    let keep = exp + 1 + decimals as i64;
    let mut kept: Vec<u8> = if keep <= 0 {
        Vec::new()
    } else {
        let k = keep as usize;
        let mut v: Vec<u8> = digits.iter().copied().take(k).collect();
        v.resize(k, b'0');
        v
    };
    let next = if keep < 0 { b'0' } else { digits.get(keep as usize).copied().unwrap_or(b'0') };
    if next >= b'5' {
        // Half up: add one in the last kept place.
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    // At least one digit before the point.
    while kept.len() < decimals + 1 {
        kept.insert(0, b'0');
    }
    let split = kept.len() - decimals;
    let mut out = String::with_capacity(kept.len() + 2);
    // Java keeps the sign of a negative number that rounds to zero ("-0"), and of -0.0.
    if x.is_sign_negative() {
        out.push('-');
    }
    let int_part = std::str::from_utf8(&kept[..split]).expect("ascii digits");
    let trimmed = int_part.trim_start_matches('0');
    out.push_str(if trimmed.is_empty() { "0" } else { trimmed });
    if decimals > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&kept[split..]).expect("ascii digits"));
    }
    out
}

/// A yield as `/nuke` prints it (Java: `NukeCommand.fmt`): "450 t", "12.4 kt", "1.05 Mt".
pub fn nuke_yield(kt: f64) -> String {
    if kt >= 1000.0 {
        format!("{} Mt", fixed(kt / 1000.0, 2))
    } else if kt < 1.0 {
        format!("{} t", fixed(kt * 1000.0, 0))
    } else {
        format!("{} kt", fixed(kt, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::fixed;

    #[test]
    fn rounds_the_shortest_decimal_half_up_like_java() {
        assert_eq!(fixed(2.5, 0), "3");
        assert_eq!(fixed(12.35, 1), "12.4");
        assert_eq!(fixed(0.125, 2), "0.13");
        assert_eq!(fixed(1.255, 2), "1.26");
        assert_eq!(fixed(1.95, 1), "2.0");
        assert_eq!(fixed(99.95, 1), "100.0");
        assert_eq!(fixed(1234.0, 0), "1234");
        assert_eq!(fixed(0.0, 2), "0.00");
        assert_eq!(fixed(0.004, 2), "0.00");
        assert_eq!(fixed(0.005, 2), "0.01");
        assert_eq!(fixed(0.0004, 0), "0");
        assert_eq!(fixed(-1.5, 0), "-2");
        assert_eq!(fixed(1.0e-7, 3), "0.000");
    }
}
