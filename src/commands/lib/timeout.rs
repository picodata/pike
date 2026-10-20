use std::{
    cmp::Ordering,
    fmt::{self, Display, Formatter},
    num::ParseIntError,
    ops::{Deref, Div},
    str::FromStr,
    time::Duration,
    write,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timeout(Duration);

impl Timeout {
    pub const fn from_secs(s: u64) -> Self {
        Self(Duration::from_secs(s))
    }

    pub fn parse_secs(s: &str) -> Result<Self, ParseIntError> {
        s.parse::<u64>().map(Self::from_secs)
    }
}

impl FromStr for Timeout {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_secs(s)
    }
}

impl Deref for Timeout {
    type Target = Duration;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Timeout> for Duration {
    fn from(val: Timeout) -> Self {
        val.0
    }
}

impl From<Duration> for Timeout {
    fn from(d: Duration) -> Self {
        Self(d)
    }
}

impl Display for Timeout {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_secs())
    }
}

impl PartialOrd<Duration> for Timeout {
    fn partial_cmp(&self, other: &Duration) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl PartialOrd<Timeout> for Duration {
    fn partial_cmp(&self, other: &Timeout) -> Option<Ordering> {
        self.partial_cmp(&other.0)
    }
}

impl PartialEq<Duration> for Timeout {
    fn eq(&self, other: &Duration) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Timeout> for Duration {
    fn eq(&self, other: &Timeout) -> bool {
        *self == other.0
    }
}

impl Div<u32> for Timeout {
    type Output = Timeout;

    fn div(self, rhs: u32) -> Self::Output {
        Self(self.0 / rhs)
    }
}
