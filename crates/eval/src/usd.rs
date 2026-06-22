use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Div, Mul, Sub};
use std::str::FromStr;

/// A USD dollar amount backed by `rust_decimal::Decimal` for exact decimal arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Usd(Decimal);

impl Usd {
    pub fn zero() -> Self {
        Usd(Decimal::ZERO)
    }

    pub fn round_dp(self, dp: u32) -> Self {
        Usd(self.0.round_dp(dp))
    }

    /// Returns `self / rhs` as a dimensionless `f64` ratio.
    pub fn ratio_as_f64(self, rhs: Usd) -> f64 {
        (self.0 / rhs.0).to_f64().unwrap_or(0.0)
    }

    /// Returns `floor(self / rhs)` as a `u64` — useful for deriving item counts from a budget.
    pub fn ratio_floor(self, rhs: Usd) -> u64 {
        (self.0 / rhs.0).floor().to_u64().unwrap_or(0)
    }
}

impl fmt::Display for Usd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.4}", self.0)
    }
}

impl FromStr for Usd {
    type Err = rust_decimal::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str(s).map(Usd)
    }
}

impl TryFrom<f64> for Usd {
    type Error = rust_decimal::Error;
    fn try_from(v: f64) -> Result<Self, Self::Error> {
        Decimal::try_from(v).map(Usd)
    }
}

impl Add for Usd {
    type Output = Usd;
    fn add(self, rhs: Usd) -> Usd {
        Usd(self.0 + rhs.0)
    }
}

impl Sub for Usd {
    type Output = Usd;
    fn sub(self, rhs: Usd) -> Usd {
        Usd(self.0 - rhs.0)
    }
}

impl Mul<u64> for Usd {
    type Output = Usd;
    fn mul(self, rhs: u64) -> Usd {
        Usd(self.0 * Decimal::from(rhs))
    }
}

impl Div<u64> for Usd {
    type Output = Usd;
    fn div(self, rhs: u64) -> Usd {
        Usd(self.0 / Decimal::from(rhs))
    }
}
