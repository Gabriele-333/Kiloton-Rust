//! Yield of an implosion fission device from what is loaded into it (Java: `FissionModel`).
//!
//! The pit is built from cores of weapons-grade uranium (90% U-235) or plutonium (Pu-239). Each
//! material contributes to the criticality in proportion to its mass over its critical mass in the
//! imploded state: alpha = sum(m / Mc). A depleted-uranium tamper reflects neutrons back (lower
//! critical mass) and adds fast fissions of U-238 (more yield).
//!
//! Below alpha = 1 the assembly never goes supercritical: only the explosive lenses go off. Above it
//! the fraction of the pit that fissions grows with the excess criticality,
//! efficiency = E_max (1 - 1/alpha)^2, a smooth stand-in for Serber's estimate. Complete fission of
//! one kilogram releases about 17.5 kt.
//!
//! Calibration: 6 kg of plutonium in a tamper (Fat Man) gives about 20 kt; the full pit of uranium
//! with a tamper about 115 kt.

use crate::format::fixed;

/// Energy of complete fission, kt of TNT per kilogram.
pub const KT_PER_KG: f64 = 17.5;
pub const MAX_EFFICIENCY: f64 = 0.5;
/// The tamper multiplies the critical mass by this.
pub const TAMPER_CRITICAL_FACTOR: f64 = 0.65;
/// Extra yield from fast fissions in the U-238 tamper.
pub const TAMPER_YIELD_BONUS: f64 = 0.2;
/// Cores the pit takes.
pub const PIT_SLOTS: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// 90% U-235.
    Uranium,
    /// Weapons-grade Pu-239.
    Plutonium,
}

impl Material {
    /// Mass of one core, kg.
    pub fn kg_per_core(self) -> f64 {
        match self {
            Material::Uranium => 4.0,
            Material::Plutonium => 2.0,
        }
    }

    /// Critical mass of a bare sphere once the lenses have compressed it, kg.
    pub fn critical_mass_kg(self) -> f64 {
        match self {
            Material::Uranium => 12.0,
            Material::Plutonium => 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FissionState {
    /// Nothing in the pit: the lenses only.
    Empty,
    /// Some material, never critical: the lenses only, the pit is scattered.
    Subcritical,
    /// Barely supercritical: a nuclear yield, but a small one.
    Fizzle,
    Nominal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FissionModel {
    pub uranium_cores: i32,
    pub plutonium_cores: i32,
    pub tamper: bool,
    pub mass_kg: f64,
    /// Multiples of the critical mass (k-effective stand-in).
    pub alpha: f64,
    pub efficiency: f64,
    /// 0 when nothing but the lenses goes off.
    pub yield_kt: f64,
    pub state: FissionState,
}

impl FissionModel {
    pub fn new(uranium_cores: i32, plutonium_cores: i32, tamper: bool) -> Self {
        let uranium_cores = uranium_cores.max(0);
        let plutonium_cores = plutonium_cores.max(0);
        let mu = uranium_cores as f64 * Material::Uranium.kg_per_core();
        let mp = plutonium_cores as f64 * Material::Plutonium.kg_per_core();
        let factor = if tamper { TAMPER_CRITICAL_FACTOR } else { 1.0 };
        let mass_kg = mu + mp;
        let alpha = mu / (Material::Uranium.critical_mass_kg() * factor)
            + mp / (Material::Plutonium.critical_mass_kg() * factor);
        let efficiency = if alpha <= 1.0 {
            0.0
        } else {
            let x = 1.0 - 1.0 / alpha;
            MAX_EFFICIENCY * x * x
        };
        let bonus = if tamper && efficiency > 0.0 { 1.0 + TAMPER_YIELD_BONUS } else { 1.0 };
        let yield_kt = mass_kg * KT_PER_KG * efficiency * bonus;
        let state = if mass_kg <= 0.0 {
            FissionState::Empty
        } else if alpha <= 1.0 {
            FissionState::Subcritical
        } else if yield_kt < 1.0 {
            FissionState::Fizzle
        } else {
            FissionState::Nominal
        };
        Self { uranium_cores, plutonium_cores, tamper, mass_kg, alpha, efficiency, yield_kt, state }
    }

    pub fn nuclear(&self) -> bool {
        self.yield_kt > 0.0
    }

    /// "450 t", "12.4 kt", "1.05 Mt".
    pub fn format(kt: f64) -> String {
        if kt >= 1000.0 {
            format!("{} Mt", fixed(kt / 1000.0, 2))
        } else if kt < 1.0 {
            format!("{} t", fixed(kt * 1000.0, 0))
        } else if kt < 100.0 {
            format!("{} kt", fixed(kt, 1))
        } else {
            format!("{} kt", fixed(kt, 0))
        }
    }

    /// "1.9 km" or "640 m".
    pub fn distance(m: f64) -> String {
        if m >= 1000.0 {
            format!("{} km", fixed(m / 1000.0, 1))
        } else {
            format!("{} m", fixed(m, 0))
        }
    }
}
