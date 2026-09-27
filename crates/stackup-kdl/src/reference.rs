use std::fmt;

/// A reference to something in a design: a placement, a pin, a port, a line, a pad.
///
/// The grammar is `path[.member…][@pad]`, where the path is names separated by `/`:
///
/// | Separator | Selects |
/// |---|---|
/// | `/` | a level of hierarchy — `timer/u` |
/// | `.` | a pin, port or line of what precedes it — `u.OUT`, `reg.out.rail` |
/// | `@` | a pad of a pin, by its label — `VDD@4`, `mcu.VDD@A1` |
///
/// A reference is syntax only. Whether `out` is a port or a pin, and whether it exists, is the
/// design's question, not the reader's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Reference {
    /// The hierarchical part, one name per level. Never empty.
    pub path: Vec<String>,
    /// What is selected on it, outermost first.
    pub members: Vec<String>,
    /// A pad label, when one is selected.
    pub pad: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceError {
    pub message: String,
    /// Byte offset into the reference text.
    pub at: usize,
}

impl Reference {
    /// A reference to one name, with nothing selected on it.
    pub fn name(name: impl Into<String>) -> Self {
        Reference {
            path: vec![name.into()],
            members: Vec::new(),
            pad: None,
        }
    }

    pub fn parse(text: &str) -> Result<Self, ReferenceError> {
        #[derive(PartialEq)]
        enum State {
            Path,
            Members,
            Pad,
        }
        let mut r = Reference {
            path: Vec::new(),
            members: Vec::new(),
            pad: None,
        };
        let mut state = State::Path;
        let mut start = 0;
        let flush = |r: &mut Reference, state: &State, start: usize, end: usize| {
            let piece = &text[start..end];
            if piece.is_empty() {
                return Err(ReferenceError {
                    message: "a reference has an empty part".into(),
                    at: start,
                });
            }
            match state {
                State::Path => r.path.push(piece.into()),
                State::Members => r.members.push(piece.into()),
                State::Pad => r.pad = Some(piece.into()),
            }
            Ok(())
        };
        for (i, c) in text.char_indices() {
            match (c, &state) {
                ('/', State::Path) => {
                    flush(&mut r, &state, start, i)?;
                    start = i + 1;
                }
                ('/', State::Members) => {
                    return Err(ReferenceError {
                        message: "`/` can't follow `.`: the hierarchy comes first".into(),
                        at: i,
                    });
                }
                ('.', State::Path | State::Members) => {
                    flush(&mut r, &state, start, i)?;
                    state = State::Members;
                    start = i + 1;
                }
                ('@', State::Path | State::Members) => {
                    flush(&mut r, &state, start, i)?;
                    state = State::Pad;
                    start = i + 1;
                }
                ('/' | '.' | '@', State::Pad) => {
                    return Err(ReferenceError {
                        message: format!("`{c}` can't follow `@`: a pad label ends a reference"),
                        at: i,
                    });
                }
                _ => {}
            }
        }
        flush(&mut r, &state, start, text.len())?;
        Ok(r)
    }

    /// The last name on the path: what the reference is *of*, before any selection.
    pub fn leaf(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or("")
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path.join("/"))?;
        for m in &self.members {
            write!(f, ".{m}")?;
        }
        if let Some(pad) = &self.pad {
            write!(f, "@{pad}")?;
        }
        Ok(())
    }
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReferenceError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(s: &str) -> (Vec<String>, Vec<String>, Option<String>) {
        let r = Reference::parse(s).unwrap();
        assert_eq!(r.to_string(), s, "round trip");
        (r.path, r.members, r.pad)
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn grammar() {
        assert_eq!(parts("u"), (strings(&["u"]), vec![], None));
        assert_eq!(parts("timer/u"), (strings(&["timer", "u"]), vec![], None));
        assert_eq!(parts("u.OUT"), (strings(&["u"]), strings(&["OUT"]), None));
        assert_eq!(
            parts("reg.out.rail"),
            (strings(&["reg"]), strings(&["out", "rail"]), None)
        );
        assert_eq!(
            parts("VDD@4"),
            (strings(&["VDD"]), vec![], Some("4".into()))
        );
        assert_eq!(
            parts("mcu.VDD@A1"),
            (strings(&["mcu"]), strings(&["VDD"]), Some("A1".into()))
        );
        assert_eq!(parts("Cvdd/1"), (strings(&["Cvdd", "1"]), vec![], None));
        assert_eq!(parts("J.1"), (strings(&["J"]), strings(&["1"]), None));
    }

    #[test]
    fn errors() {
        assert!(Reference::parse("a/").is_err());
        assert!(Reference::parse("a..b").is_err());
        assert!(Reference::parse("a.b/c").is_err());
        assert!(Reference::parse("a@1.b").is_err());
        assert!(Reference::parse("").is_err());
    }
}
