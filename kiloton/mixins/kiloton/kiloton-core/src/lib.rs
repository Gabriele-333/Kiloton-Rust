//! Kiloton's model of a nuclear burst: physics and the shapes the client draws, as pure math with no
//! game types. The same numbers as the Java edition's `common` module, function for function, so a
//! burst does the same thing on every build. One block is one metre; times are in seconds unless a
//! name says otherwise.

pub mod blast;
pub mod config;
pub mod format;
pub mod fx;
pub mod machine;
pub mod physics;

pub use physics::{
    ballistics::Ballistics,
    black_rain::BlackRain,
    blast::BlastModel,
    burst_detection::BurstDetection,
    fallout,
    fission::{FissionModel, FissionState, Material},
    intercept::Intercept,
    radar_horizon,
    sam_geometry,
    thermonuclear::{ThermonuclearModel, ThermonuclearState},
    warhead::Warhead,
};
