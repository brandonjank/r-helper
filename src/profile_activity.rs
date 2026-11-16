//! Session history, not an assertion that external software has left settings unchanged.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Activity {
    pub name: String,
    pub reason: String,
    pub areas: String,
}
impl Default for Activity {
    fn default() -> Self {
        Self::new(
            "No profile applied this session",
            "Current hardware settings are shown when read",
            "",
        )
    }
}
impl Activity {
    pub(crate) fn setting(change: &librazer::blade18::Change) -> Self {
        use librazer::blade18::Change;
        let area = match change {
            Change::Brightness(_) => "keyboard brightness",
            Change::Logo(_) => "logo",
            Change::BatteryCare(_) => "charge limit",
            Change::Performance(_) | Change::Boosts(..) => "performance",
            Change::ManualFans(_) | Change::ManualFanZones(_) | Change::AutoFans => "fans",
        };
        Self::new("Manual settings", "Applied from R-Helper", area)
    }
    pub(crate) fn new(
        name: impl Into<String>,
        reason: impl Into<String>,
        areas: impl Into<String>,
    ) -> Self {
        Self { name: name.into(), reason: reason.into(), areas: areas.into() }
    }
    pub(crate) fn summary(&self) -> String {
        if self.areas.is_empty() {
            self.name.clone()
        } else {
            format!("Last applied: {} · {}", self.name, self.areas)
        }
    }
    pub(crate) fn power(
        ac: bool,
        settings: &crate::blade_profiles::Settings,
        reason: &str,
    ) -> Self {
        let (display, lighting) = if ac {
            (&settings.display_ac, &settings.lighting_ac)
        } else {
            (&settings.display_battery, &settings.lighting_battery)
        };
        let mut areas = vec!["performance, fans, charge limit, keyboard brightness + logo"];
        if display.is_some() {
            areas.push("display");
        }
        if lighting.is_some() {
            areas.push("keyboard effects + underside");
        }
        Self::new(if ac { "AC profile" } else { "Battery profile" }, reason, areas.join(" · "))
    }
    pub(crate) fn named(profile: &crate::blade_profiles::Named, reason: &str) -> Self {
        Self::new(&profile.name, reason, "performance, fans, charge limit · lighting · display")
    }
}
