use super::*;
static LAST: std::sync::Mutex<Option<Preset>> = std::sync::Mutex::new(None);
fn cache_path() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| std::path::PathBuf::from(p).join("RHelper").join("last-lighting.json"))
}
fn cached() -> Option<Preset> {
    let path = cache_path()?;
    if std::fs::metadata(&path).ok()?.len() > 65536 {
        return None;
    }
    let p: Preset = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    p.validate().ok()?;
    Some(p)
}
fn persist(preset: &Preset) -> Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
    ));
    let mut file = File::options().write(true).create_new(true).open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, preset)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    Off,
    Static,
    Breathing,
    DualBreathing,
    WaveLeft,
    WaveRight,
    Reactive,
    Starlight,
    Ripple,
    PerKey,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub style: Style,
    pub color: [u8; 3],
    pub second: [u8; 3],
    pub keys: Vec<[u8; 3]>,
    pub brightness: u8,
    pub underside: bool,
    pub underside_brightness: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underside_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underside_keys: Option<Vec<[u8; 3]>>,
    pub logo: crate::types::LogoMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u8>,
}
impl Preset {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.underside_color.is_none() || matches!(self.style, Style::Static | Style::PerKey),
            "Independent underside color requires Static or Per-key keyboard lighting"
        );
        ensure!(
            self.underside_keys
                .as_ref()
                .is_none_or(|keys| keys.len() == 19 && self.underside_color.is_some()),
            "Underside map requires independent color and exactly 19 cells"
        );
        ensure!(self.keys.len() == 114, "Keyboard map must contain 6 × 19 colors");
        if let Some(speed) = self.speed {
            ensure!(
                match self.style {
                    Style::WaveLeft | Style::WaveRight => speed == 40,
                    Style::Reactive => (1..=4).contains(&speed),
                    Style::Starlight => (1..=3).contains(&speed),
                    Style::Ripple => speed == 2,
                    _ => false,
                },
                "Timing value is not supported for this effect"
            );
        }
        Ok(())
    }
    fn packet(&self) -> Vec<u8> {
        let (id, flags, rate, colors) = match self.style {
            Style::Off => (0, 0, 0, vec![]),
            Style::Static => (1, 0, 1, vec![self.color]),
            Style::Breathing => (2, 0, 1, vec![self.color]),
            Style::DualBreathing => (2, 0, 1, vec![self.color, self.second]),
            // Directions verified visually on the Blade 18 from the typing position.
            Style::WaveLeft => (4, 2, self.speed.unwrap_or(40), vec![]),
            Style::WaveRight => (4, 1, self.speed.unwrap_or(40), vec![]),
            Style::Reactive => (5, 0, self.speed.unwrap_or(2), vec![self.color]),
            Style::Starlight => (7, 0, self.speed.unwrap_or(2), vec![self.color, self.second]),
            Style::Ripple => (6, 0, 2, vec![self.color]),
            Style::PerKey => (8, 0, 0, vec![]),
        };
        let mut p = vec![1, 5, id, flags, rate, colors.len() as u8];
        for color in colors {
            p.extend_from_slice(&color);
        }
        p
    }
    pub fn capture(s: &Lighting, logo: crate::types::LogoMode) -> Result<Self> {
        if super::native::active() {
            if let Some(mut preset) = LAST
                .lock()
                .map_err(|_| anyhow::anyhow!("Lighting cache unavailable"))?
                .clone()
                .filter(|p| p.underside_color.is_some())
            {
                preset.brightness = s.keyboard_brightness;
                preset.underside = s.underside;
                preset.underside_brightness = s.underside_brightness;
                preset.logo = logo;
                ensure!(
                    preset.matches(s),
                    "Lighting changed outside R-Helper; native pixels cannot be captured"
                );
                return Ok(preset);
            }
        }
        let d = &s.keyboard_effect;
        if d.get(2) == Some(&8) {
            let mut p=LAST.lock().map_err(|_|anyhow::anyhow!("Lighting cache unavailable"))?.clone().or_else(cached).filter(|p|p.style==Style::PerKey).context("External Chroma pixels cannot be captured; apply an R-Helper firmware effect first")?;
            p.brightness = s.keyboard_brightness;
            p.underside = s.underside;
            p.underside_brightness = s.underside_brightness;
            p.logo = logo;
            ensure!(p.matches(s), "Current per-key mode differs from the last R-Helper map");
            return Ok(p);
        }
        let style = match (d.get(2), d.get(5)) {
            (Some(0), Some(0)) => Style::Off,
            (Some(1), Some(1)) => Style::Static,
            (Some(2), Some(1)) => Style::Breathing,
            (Some(2), Some(2)) => Style::DualBreathing,
            (Some(4), Some(0)) => {
                if d[3] == 1 {
                    Style::WaveRight
                } else {
                    Style::WaveLeft
                }
            }
            (Some(5), Some(1)) => Style::Reactive,
            (Some(7), Some(2)) => Style::Starlight,
            (Some(6), Some(1)) => Style::Ripple,
            _ => anyhow::bail!(
                "Current Chroma pixels cannot be captured; apply an R-Helper effect first"
            ),
        };
        let color = if d.len() >= 9 { [d[6], d[7], d[8]] } else { [32, 64, 255] };
        let preset = Self {
            style,
            color,
            second: if d.len() >= 12 { [d[9], d[10], d[11]] } else { [255, 64, 32] },
            keys: vec![color; 114],
            brightness: s.keyboard_brightness,
            underside: s.underside,
            underside_brightness: s.underside_brightness,
            underside_color: None,
            underside_keys: None,
            logo,
            speed: if matches!(
                style,
                Style::WaveLeft | Style::WaveRight | Style::Reactive | Style::Starlight
            ) {
                Some(d[4])
            } else {
                None
            },
        };
        preset.validate()?;
        ensure!(
            preset.matches(s),
            "Current effect parameters cannot be captured exactly; apply an R-Helper effect first"
        );
        Ok(preset)
    }
    fn matches(&self, s: &Lighting) -> bool {
        let expected = self.packet();
        let mut actual = s.keyboard_effect.clone();
        if matches!(self.style, Style::Static | Style::Breathing | Style::DualBreathing)
            && actual.get(4).is_some_and(|v| *v <= 1)
        {
            actual[4] = 1;
        }
        actual == expected
            && s.keyboard_brightness == self.brightness
            && s.underside == self.underside
            && s.underside_brightness == self.underside_brightness
    }
    fn write(&self, device: &Device) -> Result<()> {
        super::native::mode_write(
            device,
            if self.underside_color.is_some() { NativeMode::Driver } else { NativeMode::Firmware },
        )?;
        if self.underside_color.is_some() {
            write(device, Change::Underside(false))?;
        }
        c::send_command(device, 0x0f02, &self.packet())?;
        if self.style == Style::PerKey && self.underside_color.is_none() {
            for row in 0..6 {
                let mut p = vec![0, 5, row as u8, 0, 18];
                for color in &self.keys[row * 19..(row + 1) * 19] {
                    p.extend_from_slice(color);
                }
                c::send_command(device, 0x0f03, &p)?;
            }
        }
        c::set_keyboard_brightness(device, self.brightness)?;
        c::set_logo_mode(device, self.logo)?;
        write(device, Change::UndersideBrightness(self.underside_brightness))?;
        if let Some(color) = self.underside_color {
            let keyboard =
                if self.style == Style::PerKey { self.keys.clone() } else { vec![self.color; 114] };
            let underside = self.underside_keys.clone().unwrap_or_else(|| vec![color; 19]);
            super::native::write_pixels(device, &keyboard, &underside)?;
        }
        write(device, Change::Underside(self.underside))?;
        Ok(())
    }
    pub fn apply(&self, path: &Path) -> Result<Lighting> {
        self.validate()?;
        super::super::lamp_array::release();
        let mut file = File::options().create_new(true).write(true).open(path)?;
        let device = open()?;
        let before = read(&device)?;
        let logo = c::get_logo_mode(&device)?;
        // For CustomFrame, the caller must retain the last applied R-Helper map
        // for recovery; never pretend effect metadata backs up the pixels.
        let original_mode = super::native::mode_on(&device)?;
        // Another driver's native pixels are not backed up by stored effect metadata.
        let recovery = if original_mode == NativeMode::Driver && !super::native::active() {
            None
        } else {
            Self::capture(&before, logo).ok()
        };
        if self.underside_color.is_some() {
            super::native::arm(recovery.as_ref(), original_mode)?;
        }
        super::super::journal(
            &mut file,
            &serde_json::json!({"model":"RZ09-0529","operation":"lighting_preset","original":before,"recovery":recovery,"requested":self}),
        )?;
        let result = self.write(&device).and_then(|_| {
            let after = read(&device)?;
            ensure!(
                self.matches(&after)
                    && c::get_logo_mode(&device)? == self.logo
                    && before.startup_animation == after.startup_animation,
                "Lighting preset readback mismatch: {:?}",
                after.keyboard_effect
            );
            Ok(after)
        });
        let rollback = if result.is_err() {
            recovery.as_ref().map(|p| {
                p.write(&device).and_then(|_| {
                    ensure!(p.matches(&read(&device)?), "Lighting rollback mismatch");
                    Ok(())
                })
            })
        } else {
            None
        };
        super::super::journal(
            &mut file,
            &serde_json::json!({"model":"RZ09-0529","operation":"lighting_preset","original":before,"recovery":recovery,"requested":self,"result":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{e:#}")),"rollback":rollback.as_ref().map(|r|r.as_ref().map(|_|true).map_err(|e|format!("{e:#}")))}),
        )?;
        if let Some(Err(error)) = rollback {
            return Err(error.context("Lighting recovery failed"));
        }
        if result.is_ok() {
            super::native::set_active(self.underside_color.is_some());
            *LAST.lock().map_err(|_| anyhow::anyhow!("Lighting cache unavailable"))? =
                Some(self.clone());
            persist(self)
                .context("Lighting applied, but its local preset cache could not be saved")?;
        }
        result
    }
}
impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Static => "Static",
            Self::Breathing => "Breathing",
            Self::DualBreathing => "Two-color breathing",
            Self::WaveLeft => "Wave left",
            Self::WaveRight => "Wave right",
            Self::Reactive => "Reactive",
            Self::Starlight => "Starlight",
            Self::Ripple => "Ripple",
            Self::PerKey => "Per-key colors",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_only_exact_recoverable_firmware_parameters() {
        let mut s = Lighting {
            startup_animation: true,
            underside: false,
            underside_brightness: 64,
            keyboard_effect: vec![1, 5, 2, 0, 0, 2, 32, 64, 255, 255, 64, 32],
            logo_effect: vec![1, 4, 1, 0, 0, 0],
            keyboard_brightness: 176,
            logo_brightness: 255,
        };
        let p = Preset::capture(&s, crate::types::LogoMode::Static).unwrap();
        assert_eq!(p.style, Style::DualBreathing);
        assert_eq!(p.second, [255, 64, 32]);
        s.keyboard_effect = vec![1, 5, 4, 1, 40, 0];
        let right = Preset::capture(&s, crate::types::LogoMode::Static).unwrap();
        assert_eq!(right.style, Style::WaveRight);
        assert_eq!(right.packet(), s.keyboard_effect);
        s.keyboard_effect[3] = 2;
        let left = Preset::capture(&s, crate::types::LogoMode::Static).unwrap();
        assert_eq!(left.style, Style::WaveLeft);
        assert_eq!(left.packet(), s.keyboard_effect);
        s.keyboard_effect[4] = 99;
        assert!(Preset::capture(&s, crate::types::LogoMode::Static).is_err());
        s.keyboard_effect = vec![1, 5, 3, 0, 0, 0];
        assert!(Preset::capture(&s, crate::types::LogoMode::Static).is_err());
    }
}
