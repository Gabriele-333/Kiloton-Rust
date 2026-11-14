//! Yield of a two-stage (Teller-Ulam) thermonuclear device from what is loaded into it (Java:
//! `ThermonuclearModel`).
//!
//! The primary is an ordinary implosion fission device ([`FissionModel`]). Its X-rays implode the
//! secondary: lithium-6 deuteride round a fissile spark plug, inside a heavy pusher.
//!
//! - Fuel: one charge is 50 kg of lithium-6 deuteride, 50 kt per kilogram burnt (2.5 Mt a charge).
//!   No real device burns it all: here at most half.
//! - Primary: about 15 kt of primary for each charge. A weaker primary compresses the secondary
//!   less and less of it burns. Under 3 kt the secondary does not light at all.
//! - Spark plug: without it the fuel is lit only by the compression and most of it is blown apart.
//! - Jacket: a depleted-uranium pusher fissions under the 14 MeV fusion neutrons (the third stage),
//!   doubling the yield and making most of the fallout. Without it the device is "clean".
//!
//! Calibration: six charges with spark plug and jacket behind a full uranium primary give 15 Mt
//! (Castle Bravo). One charge behind a Fat Man class primary gives 1.25 Mt clean, 2.5 Mt jacketed.

use super::fission::FissionModel;

/// Fuel charges the secondary takes.
pub const FUEL_SLOTS: i32 = 6;
/// Lithium-6 deuteride in one charge, kg.
pub const KG_PER_CHARGE: f64 = 50.0;
/// Energy of complete burn-up of lithium-6 deuteride, kt of TNT per kilogram.
pub const KT_PER_KG: f64 = 50.0;
/// Largest fraction of the fuel that burns.
pub const MAX_BURN: f64 = 0.5;
/// Primary yield it takes to compress one charge fully, kt.
pub const PRIMARY_KT_PER_CHARGE: f64 = 15.0;
/// Under this the primary cannot implode the secondary at all, kt.
pub const IGNITION_KT: f64 = 3.0;
/// Share of the burn left when there is no spark plug.
pub const NO_SPARK_PLUG: f64 = 0.15;
/// Fission of the uranium jacket, as a multiple of the fusion yield.
pub const JACKET_FISSION: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThermonuclearState {
    /// The primary gives no nuclear yield: its lenses only.
    NoPrimary,
    /// No fuel in the secondary: the primary alone.
    NoFuel,
    /// The primary is too weak to implode the secondary: the primary alone.
    NoIgnition,
    /// The secondary lights, badly: no spark plug, or a primary too small for this much fuel.
    Partial,
    Nominal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermonuclearModel {
    pub primary: FissionModel,
    pub charges: i32,
    pub spark_plug: bool,
    pub jacket: bool,
    pub fuel_kg: f64,
    /// How well the primary compresses this much fuel, 0 .. 1.
    pub compression: f64,
    /// Fraction of the fuel that burns.
    pub burn: f64,
    pub fusion_kt: f64,
    /// Fast fission of the uranium jacket.
    pub jacket_kt: f64,
    pub yield_kt: f64,
    pub state: ThermonuclearState,
}

impl ThermonuclearModel {
    pub fn new(primary: FissionModel, charges: i32, spark_plug: bool, jacket: bool) -> Self {
        let charges = charges.clamp(0, FUEL_SLOTS);
        let fuel_kg = charges as f64 * KG_PER_CHARGE;
        let p = primary.yield_kt;
        let (compression, burn) = if !primary.nuclear() || charges == 0 || p < IGNITION_KT {
            (0.0, 0.0)
        } else {
            let compression = (p / (PRIMARY_KT_PER_CHARGE * charges as f64)).min(1.0);
            (compression, MAX_BURN * compression * if spark_plug { 1.0 } else { NO_SPARK_PLUG })
        };
        let fusion_kt = fuel_kg * KT_PER_KG * burn;
        let jacket_kt = if jacket { fusion_kt * JACKET_FISSION } else { 0.0 };
        let yield_kt = p + fusion_kt + jacket_kt;
        let state = if !primary.nuclear() {
            ThermonuclearState::NoPrimary
        } else if charges == 0 {
            ThermonuclearState::NoFuel
        } else if fusion_kt <= 0.0 {
            ThermonuclearState::NoIgnition
        } else if !spark_plug || compression < 0.999 {
            ThermonuclearState::Partial
        } else {
            ThermonuclearState::Nominal
        };
        Self {
            primary,
            charges,
            spark_plug,
            jacket,
            fuel_kg,
            compression,
            burn,
            fusion_kt,
            jacket_kt,
            yield_kt,
            state,
        }
    }

    pub fn nuclear(&self) -> bool {
        self.primary.nuclear()
    }

    /// True when the secondary lights.
    pub fn fusion(&self) -> bool {
        self.fusion_kt > 0.0
    }

    /// Share of the yield that comes from fission (primary and jacket): what makes the fallout.
    pub fn fission_fraction(&self) -> f64 {
        if self.yield_kt <= 0.0 { 1.0 } else { (self.primary.yield_kt + self.jacket_kt) / self.yield_kt }
    }

    /// Primary yield that would compress this much fuel fully, kt.
    pub fn primary_needed_kt(&self) -> f64 {
        IGNITION_KT.max(PRIMARY_KT_PER_CHARGE * self.charges as f64)
    }
}
