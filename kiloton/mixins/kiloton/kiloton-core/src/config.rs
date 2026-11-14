//! The settings and their defaults (Java: `KilotonConfig`), read from the same TOML files the Java
//! edition writes: `serverconfig/kiloton-server.toml` in the world and `kiloton-client.toml` in the
//! config folder. Only the ones something already reads are here; the rest come with the parts that
//! use them.

#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// Cap, in blocks, on how far from ground zero blocks are touched (`maxRadius`, 32..8000).
    pub max_radius: i32,
    /// Largest yield of a fission device and of `/nuke` below the thermonuclear range (`maxYieldKt`).
    pub max_yield_kt: f64,
    /// Largest yield of a thermonuclear device (`maxThermonuclearYieldKt`).
    pub max_thermonuclear_yield_kt: f64,
    /// Cap on how far a thermonuclear burst touches blocks (`maxThermonuclearRadius`, 32..120000).
    pub max_thermonuclear_radius: i32,
    /// Blast, burns and prompt radiation hurt creatures and players (`entityDamage`).
    pub entity_damage: bool,
    /// The flash sets players caught in the open on fire (`flashIgnitesPlayers`).
    pub flash_ignites_players: bool,
    /// The flash burns players caught in the open (`flashHurtsPlayers`).
    pub flash_hurts_players: bool,
    /// A survival player caught in the open by a lethal flash sees it from outside (`deathScene`).
    pub death_scene: bool,
    /// Downwind fallout that slowly poisons whoever stays in it (`fallout`).
    pub fallout: bool,
    /// Countdown a freshly placed bomb starts with, in seconds (`timerSeconds`, 5..600).
    pub timer_seconds: i32,
    /// Players within reach of an armed bomb get a chat warning, and whoever armed it the arming message
    /// (`countdownWarning`).
    pub countdown_warning: bool,
    /// Speed of the uranium machines, 1 the default pace (`machineSpeed`, 0.1..20).
    pub machine_speed: f64,
    /// Energy the machines need, as a multiple of the realistic cost (`machineEnergy`, 0.001..1000).
    pub machine_energy: f64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_radius: 4000,
            max_yield_kt: 150.0,
            max_thermonuclear_yield_kt: 20000.0,
            max_thermonuclear_radius: 60000,
            entity_damage: true,
            flash_ignites_players: true,
            flash_hurts_players: true,
            death_scene: true,
            fallout: true,
            timer_seconds: 30,
            countdown_warning: true,
            machine_speed: 1.0,
            machine_energy: 1.0,
        }
    }
}

impl ServerConfig {
    /// The radius cap of a burst: a thermonuclear one has its own, larger limit.
    pub fn max_radius(&self, thermonuclear: bool) -> i32 {
        if thermonuclear { self.max_radius.max(self.max_thermonuclear_radius) } else { self.max_radius }
    }

    /// The yield a device goes off with (Java: `KilotonConfig.clampYield(Warhead)`): a thermonuclear one has
    /// its own, larger limit.
    pub fn clamp_yield(&self, kt: f64, thermonuclear: bool) -> f64 {
        let cap = if thermonuclear { self.max_yield_kt.max(self.max_thermonuclear_yield_kt) } else { self.max_yield_kt };
        kt.min(cap).max(0.001)
    }

    /// The largest yield `/nuke` accepts.
    pub fn max_command_yield(&self) -> f64 {
        self.max_yield_kt.max(self.max_thermonuclear_yield_kt)
    }

    /// The settings in a `kiloton-server.toml`, the defaults for what it leaves out or gets wrong
    /// (ranges as the Java edition's).
    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for (k, v) in entries(text) {
            match k.as_str() {
                "maxRadius" => num(&v, 32.0, 8000.0, |x| c.max_radius = x as i32),
                "maxYieldKt" => num(&v, 0.001, 100000.0, |x| c.max_yield_kt = x),
                "maxThermonuclearYieldKt" => num(&v, 1.0, 100000.0, |x| c.max_thermonuclear_yield_kt = x),
                "maxThermonuclearRadius" => num(&v, 32.0, 120000.0, |x| c.max_thermonuclear_radius = x as i32),
                "entityDamage" => flag(&v, |x| c.entity_damage = x),
                "flashIgnitesPlayers" => flag(&v, |x| c.flash_ignites_players = x),
                "flashHurtsPlayers" => flag(&v, |x| c.flash_hurts_players = x),
                "deathScene" => flag(&v, |x| c.death_scene = x),
                "fallout" => flag(&v, |x| c.fallout = x),
                "timerSeconds" => num(&v, 5.0, 600.0, |x| c.timer_seconds = x as i32),
                "countdownWarning" => flag(&v, |x| c.countdown_warning = x),
                "machineSpeed" => num(&v, 0.1, 20.0, |x| c.machine_speed = x),
                "machineEnergy" => num(&v, 0.001, 1000.0, |x| c.machine_energy = x),
                _ => {}
            }
        }
        c
    }

    /// The file written when there is none, with the defaults.
    pub const TEMPLATE: &'static str = "# Kiloton server settings (the same keys as the Java edition's kiloton-server.toml).\n\
[blast]\nmaxRadius = 4000\nmaxYieldKt = 150.0\nmaxThermonuclearYieldKt = 20000.0\nmaxThermonuclearRadius = 60000\n\
entityDamage = true\nflashIgnitesPlayers = true\nflashHurtsPlayers = true\ndeathScene = true\nfallout = true\n\
[bomb]\ntimerSeconds = 30\ncountdownWarning = true\nmachineSpeed = 1.0\nmachineEnergy = 1.0\n";
}

/// The client's settings (Java: `KilotonConfig` client spec).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientConfig {
    /// Visual scale of the mushroom cloud (`cloudScale`, 0.05..1.5).
    pub cloud_scale: f64,
    /// Strength of the flash on screen (`flashStrength`, 0..1).
    pub flash_strength: f64,
    /// Strength of the camera shake (`shakeStrength`, 0..2).
    pub shake_strength: f64,
    /// Sky, fog and light tinted by the flash, the glow and the dust (`skyEffects`).
    pub sky_effects: bool,
    /// The Wilson cloud behind the blast wave (`wilsonCloud`, 0 off .. 2 exaggerated).
    pub wilson_cloud: f64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self { cloud_scale: 0.75, flash_strength: 1.0, shake_strength: 1.0, sky_effects: true, wilson_cloud: 1.0 }
    }
}

impl ClientConfig {
    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for (k, v) in entries(text) {
            match k.as_str() {
                "cloudScale" => num(&v, 0.05, 1.5, |x| c.cloud_scale = x),
                "flashStrength" => num(&v, 0.0, 1.0, |x| c.flash_strength = x),
                "shakeStrength" => num(&v, 0.0, 2.0, |x| c.shake_strength = x),
                "skyEffects" => flag(&v, |x| c.sky_effects = x),
                "wilsonCloud" => num(&v, 0.0, 2.0, |x| c.wilson_cloud = x),
                _ => {}
            }
        }
        c
    }

    /// The file written when there is none, with the defaults.
    pub const TEMPLATE: &'static str = "# Kiloton client settings (the same keys as the Java edition's kiloton-client.toml).\n\
cloudScale = 0.75\nflashStrength = 1.0\nshakeStrength = 1.0\nskyEffects = true\nwilsonCloud = 1.0\n";
}

/// `key = value` lines of a TOML file, sections and comments left out (all Kiloton keys are unique).
fn entries(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next().unwrap_or("").trim();
            if l.starts_with('[') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.trim().trim_matches('"').to_owned(), v.trim().trim_matches('"').to_owned()))
        })
        .collect()
}

fn num(v: &str, lo: f64, hi: f64, set: impl FnOnce(f64)) {
    if let Ok(x) = v.parse::<f64>() {
        if x.is_finite() && x >= lo && x <= hi {
            set(x);
        }
    }
}

fn flag(v: &str, set: impl FnOnce(bool)) {
    match v {
        "true" => set(true),
        "false" => set(false),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_java_file() {
        let c = ServerConfig::parse("[blast]\n\t#comment\n\tflashHurtsPlayers = false\n\tmaxRadius = 99999\n\tdeathScene = false\n");
        assert!(!c.flash_hurts_players && !c.death_scene && c.max_radius == 4000 && c.entity_damage);
        assert_eq!(ServerConfig::parse(ServerConfig::TEMPLATE), ServerConfig::default());
        let b = ServerConfig::parse("[bomb]\ntimerSeconds = 4\ncountdownWarning = false\n");
        assert!(b.timer_seconds == 30 && !b.countdown_warning);
        let m = ServerConfig::parse("[bomb]\nmachineSpeed = 2.5\nmachineEnergy = 0.0001\n");
        assert!(m.machine_speed == 2.5 && m.machine_energy == 1.0);
        assert_eq!(ServerConfig::parse("timerSeconds = 120").timer_seconds, 120);
        let d = ServerConfig::default();
        assert_eq!(d.clamp_yield(400.0, false), 150.0);
        assert_eq!(d.clamp_yield(400.0, true), 400.0);
        assert_eq!(d.clamp_yield(0.0, false), 0.001);
        assert_eq!(ClientConfig::parse(ClientConfig::TEMPLATE), ClientConfig::default());
        assert_eq!(ClientConfig::parse("cloudScale = 0.5").cloud_scale, 0.5);
    }
}
