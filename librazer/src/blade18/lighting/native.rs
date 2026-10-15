//! Fixed seven-row native frame observed in this model's own lighting-driver log.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeMode {
    Firmware,
    Driver,
}
impl NativeMode {
    fn byte(self) -> u8 {
        match self {
            Self::Firmware => 0,
            Self::Driver => 3,
        }
    }
}
pub fn native_mode() -> Result<NativeMode> {
    mode_on(&open()?)
}
pub(super) fn mode_on(device: &Device) -> Result<NativeMode> {
    let packet = c::send_command(device, 0x0084, &[0, 0])?;
    match &packet.get_args()[..2] {
        [0, 0] => Ok(NativeMode::Firmware),
        [3, 0] => Ok(NativeMode::Driver),
        value => anyhow::bail!("Unrecognized native lighting mode: {value:?}"),
    }
}
pub fn set_native_mode(mode: NativeMode, path: &Path) -> Result<()> {
    let original = native_mode()?;
    let mut file = File::options().create_new(true).write(true).open(path)?;
    super::super::journal(
        &mut file,
        &serde_json::json!({"operation":"native_lighting_mode","original":original,"requested":mode}),
    )?;
    if original != mode {
        let device = open()?;
        device
            .send(crate::packet::Packet::new(0x0004, &[mode.byte(), 0]).blade_matrix_framing()?)?;
    }
    ensure!(native_mode()? == mode, "Native lighting mode readback mismatch");
    Ok(())
}

pub(super) fn mode_write(device: &Device, mode: NativeMode) -> Result<()> {
    if mode_on(device)? != mode {
        device
            .send(crate::packet::Packet::new(0x0004, &[mode.byte(), 0]).blade_matrix_framing()?)?;
        ensure!(mode_on(device)? == mode, "Native mode readback mismatch");
    }
    Ok(())
}
static OWNER: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn configure_native_owner(path: Option<std::path::PathBuf>) -> Result<()> {
    *OWNER.lock().map_err(|_| anyhow::anyhow!("Lighting owner unavailable"))? = path;
    ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
pub(super) fn active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}
pub(super) fn set_active(value: bool) {
    ACTIVE.store(value, std::sync::atomic::Ordering::SeqCst);
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    preset: Preset,
    mode: NativeMode,
}
pub(super) fn arm(preset: Option<&Preset>, mode: NativeMode) -> Result<()> {
    let owner = OWNER.lock().map_err(|_| anyhow::anyhow!("Lighting owner unavailable"))?;
    let path = owner.as_ref().context(
        "Pause Razer software with Take control before using independent underside color",
    )?;
    if path.exists() {
        read_recovery(path)?;
        return Ok(());
    }
    let preset =
        preset.context("Current lighting cannot be backed up; apply a static effect first")?;
    ensure!(
        preset.underside_color.is_none() && mode == NativeMode::Firmware,
        "Apply a firmware effect before starting independent underside lighting"
    );
    let file = File::options().create_new(true).write(true).open(path)?;
    serde_json::to_writer_pretty(&file, &Recovery { preset: preset.clone(), mode })?;
    file.sync_all()?;
    Ok(())
}
fn read_recovery(path: &Path) -> Result<Recovery> {
    ensure!(std::fs::metadata(path)?.len() < 65536, "Native recovery record too large");
    let record: Recovery = serde_json::from_slice(&std::fs::read(path)?)?;
    record.preset.validate()?;
    ensure!(
        record.preset.underside_color.is_none()
            && record.preset.underside_keys.is_none()
            && record.mode == NativeMode::Firmware,
        "Invalid native recovery baseline"
    );
    Ok(record)
}
/// Used by the independent ownership guard before it restores vendor software.
pub fn restore_native_owner(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let record = read_recovery(path)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let mode = set_native_mode(record.mode, &path.with_extension(format!("{stamp}.mode.json")));
    let lighting = record.preset.apply(&path.with_extension(format!("{stamp}.restore.json")));
    mode?;
    lighting?;
    Ok(())
}
pub(super) fn write_static_frame(
    device: &Device,
    keyboard: [u8; 3],
    underside: [u8; 3],
) -> Result<()> {
    write_pixels(device, &[keyboard; 114], &[underside; 19])
}
pub(super) fn write_pixels(
    device: &Device,
    keyboard: &[[u8; 3]],
    underside: &[[u8; 3]],
) -> Result<()> {
    ensure!(
        keyboard.len() == 114 && underside.len() == 19,
        "Native frame must contain 114 keyboard and 19 underside cells"
    );
    for row in 0..7 {
        let colors = if row == 6 {
            underside
        } else {
            &keyboard[row as usize * 19..(row as usize + 1) * 19]
        };
        device
            .send(
                crate::packet::Packet::new(0x030b, &pixel_packet(row, colors)?)
                    .blade_matrix_framing()?,
            )
            .with_context(|| format!("Write native matrix row {row}"))?;
    }
    device.send(crate::packet::Packet::new(0x030a, &[5, 0]).blade_matrix_framing()?)?;
    Ok(())
}
#[cfg(test)]
fn row_packet(row: u8, color: [u8; 3]) -> Result<Vec<u8>> {
    pixel_packet(row, &[color; 19])
}
fn pixel_packet(row: u8, colors: &[[u8; 3]]) -> Result<Vec<u8>> {
    ensure!(row <= 6 && colors.len() == 19, "Native row must be 0–6 with exactly 19 cells");
    let mut packet = vec![0xff, row, 0, 18];
    for color in colors {
        packet.extend_from_slice(color);
    }
    packet.resize(70, 0);
    Ok(packet)
}
/// Bounded diagnostic: caller must arm restoration before entering custom mode.
/// Pixel values cannot be read back; only command acceptance/metadata are reported.
pub fn native_static_zones(
    keyboard: [u8; 3],
    underside: [u8; 3],
    path: &Path,
) -> Result<serde_json::Value> {
    let mut file = File::options().create_new(true).write(true).open(path)?;
    let device = open()?;
    let before = read(&device)?;
    let original = Preset::capture(&before, c::get_logo_mode(&device)?)?;
    ensure!(
        original.style == Style::Static,
        "Native row test requires a recoverable static effect"
    );

    super::super::journal(
        &mut file,
        &serde_json::json!({
            "model":"RZ09-0529","operation":"native_seven_row_test","original":original,
            "keyboard_rows":[0,1,2,3,4,5],"candidate_underside_row":6,"columns":19,
            "keyboard_rgb":keyboard,"underside_rgb":underside,"pixel_readback_available":false
        }),
    )?;
    let mut observed = None;
    let result = (|| -> Result<Lighting> {
        write(&device, Change::Underside(false))?;
        write_static_frame(&device, keyboard, underside)?;
        write(&device, Change::Underside(true))?;
        let after = read(&device)?;
        observed = Some(after.clone());
        ensure!(
            after.underside
                && after.underside_brightness == before.underside_brightness
                && after.keyboard_brightness == before.keyboard_brightness
                && after.logo_effect == before.logo_effect
                && after.logo_brightness == before.logo_brightness
                && after.startup_animation == before.startup_animation,
            "Native frame changed unrelated lighting metadata: {after:?}"
        );
        Ok(after)
    })();
    super::super::journal(
        &mut file,
        &serde_json::json!({"model":"RZ09-0529","operation":"native_seven_row_test","original":original,"keyboard_rgb":keyboard,"underside_rgb":underside,"candidate_underside_row":6,"accepted":result.is_ok(),"after":observed,"error":result.as_ref().err().map(|e|format!("{e:#}")),"visual_confirmation_required":true}),
    )?;
    result?;
    Ok(
        serde_json::json!({"accepted":true,"protocol":"native_030b","candidate_underside_row":6,"keyboard_rgb":keyboard,"underside_rgb":underside,"pixel_readback_available":false}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pixel_packets_keep_column_order_and_reject_partial_rows() {
        let mut colors = vec![[0, 0, 0]; 19];
        colors[0] = [255, 0, 0];
        colors[18] = [0, 255, 0];
        let packet = pixel_packet(6, &colors).unwrap();
        assert_eq!(&packet[4..7], &[255, 0, 0]);
        assert_eq!(&packet[58..61], &[0, 255, 0]);
        assert!(pixel_packet(6, &colors[..18]).is_err());
        assert!(pixel_packet(7, &colors).is_err());
    }
    #[test]
    fn native_row_layout_is_exact_and_excludes_other_regions_and_rows() {
        for row in 0..=6 {
            let p = row_packet(row, [255, 32, 32]).unwrap();
            assert_eq!(&p[..4], &[0xff, row, 0, 18]);
            assert_eq!(p.len(), 70);
            assert!(p[4..61].chunks_exact(3).all(|rgb| rgb == [255, 32, 32]));
            assert_eq!(&p[61..], &[0; 9]);
        }
        assert!(row_packet(7, [0; 3]).is_err());
        assert!(row_packet(255, [0; 3]).is_err());
    }
}
