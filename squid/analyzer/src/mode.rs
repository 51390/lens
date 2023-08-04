use std::fmt::{Display,Formatter,Result};

pub enum Mode {
    REQMOD,
    RESPMOD,
    UNKNOWN,
}

impl From<i64> for Mode {
    fn from(value: i64) -> Self {
        match value {
            0 => Mode::REQMOD,
            1 => Mode::RESPMOD,
            _ => Mode::UNKNOWN,
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let value = match self {
            Mode::REQMOD => "0",
            Mode::RESPMOD => "1",
            Mode::UNKNOWN => "UNKNOWN",
        };
        write!(f, "{}", value)
    }
}


