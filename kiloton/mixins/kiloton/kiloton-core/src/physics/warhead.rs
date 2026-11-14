//! What a loaded device will do when it goes off, whatever its kind (Java: `Warhead`): the yield,
//! how much of it is fission (the fallout follows the fission, not the total), and whether a fusion
//! stage lights (a thermonuclear burst has its own, larger limits on yield and radius).

use super::fission::{FissionModel, FissionState};
use super::thermonuclear::ThermonuclearModel;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warhead {
    /// 0 when nothing but the explosive lenses goes off.
    pub yield_kt: f64,
    pub fission_fraction: f64,
    pub thermonuclear: bool,
    /// A nuclear yield, but a small one: the pit was barely critical.
    pub fizzle: bool,
}

impl Warhead {
    pub const NONE: Warhead = Warhead { yield_kt: 0.0, fission_fraction: 1.0, thermonuclear: false, fizzle: false };

    pub fn new(yield_kt: f64, fission_fraction: f64, thermonuclear: bool, fizzle: bool) -> Self {
        Self {
            yield_kt: yield_kt.max(0.0),
            fission_fraction: fission_fraction.clamp(0.0, 1.0),
            thermonuclear,
            fizzle,
        }
    }

    pub fn of_fission(m: &FissionModel) -> Self {
        if m.nuclear() { Self::new(m.yield_kt, 1.0, false, m.state == FissionState::Fizzle) } else { Self::NONE }
    }

    pub fn of_thermonuclear(m: &ThermonuclearModel) -> Self {
        if !m.nuclear() {
            return Self::NONE;
        }
        if !m.fusion() {
            return Self::of_fission(&m.primary);
        }
        Self::new(m.yield_kt, m.fission_fraction(), true, false)
    }

    /// A burst of a given yield with nothing known about the device (commands): above the fission
    /// cap it is thermonuclear, half fission and half fusion.
    pub fn of_yield(kt: f64, fission_cap_kt: f64) -> Self {
        if kt > fission_cap_kt { Self::new(kt, 0.5, true, false) } else { Self::new(kt, 1.0, false, false) }
    }

    pub fn nuclear(&self) -> bool {
        self.yield_kt > 0.0
    }
}
