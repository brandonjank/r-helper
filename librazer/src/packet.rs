use anyhow::{ensure, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

/// Packet is the structure of the packet that is sent to the Razer HID device and received back.
/// Source https://github.com/Razer-Linux/razer-laptop-control-no-dkms/blob/main/razer_control_gui/src/device.rs.
#[repr(C)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Packet {
    status: u8,
    id: u8,
    remaining_packets: u16,
    protocol_type: u8,
    data_size: u8,
    command_class: u8,
    command_id: u8,
    #[serde(with = "BigArray")]
    args: [u8; 80],
    crc: u8,
    reserved: u8,
}

enum CommandStatus {
    New = 0x00,
    Successful = 0x02,
    NotSupported = 0x05,
}

impl Packet {
    /// Framing used by source-backed HID implementations; diagnostic opt-in only.
    pub(crate) fn standard_framing(mut self) -> Self {
        self.id = 0x1f;
        let bytes = bincode::serialize(&self).unwrap();
        self.crc = bytes[2..88].iter().fold(0, |crc, byte| crc ^ byte);
        self
    }
    pub(crate) fn ensure_fan_lab_access(&self) -> Result<()> {
        let command = u16::from_be_bytes([self.command_class, self.command_id]);
        let args = &self.args[..usize::from(self.data_size).min(80)];
        if command == 0x0d01 && matches!(args, [0 | 1, 1 | 2, 0..=55] | [0 | 1, 3, 0..=35]) {
            return Ok(());
        }
        // Source-backed alternative from kgswx/R-helper RazerEC.SetPowerRaw.
        // Limit this experiment to Balanced mode on existing primary selectors.
        if command == 0x0d02 && matches!(args, [0, 1 | 2, 0, 0 | 1]) {
            return Ok(());
        }
        self.ensure_blade18_access()
    }
    pub(crate) fn ensure_blade18_access(&self) -> Result<()> {
        if self.ensure_brightness_access().is_ok() {
            return Ok(());
        }
        let command = u16::from_be_bytes([self.command_class, self.command_id]);
        let args = &self.args[..usize::from(self.data_size).min(80)];
        let allowed = match command {
            // Exact model-711 callers: startup uses profile 0; underside uses class 1.
            0x0f18 => matches!(args, [0, 0 | 1]),
            // Bounded firmware static keyboard RGB, profile 1 / region 5 only.
            0x0f02 => matches!(args, [1, 5, 1 | 2, 0, 1, 1, _, _, _]),
            0x0300 if matches!(args, [1, 38, 0 | 1]) => true,
            0x0303 => matches!(args, [1, 38, _]),
            0x0300 => matches!(args, [1, 4, 0 | 1]),
            0x0302 => matches!(args, [1, 4, 0 | 2]),
            0x0712 => matches!(args, [0x50 | 0xd0]),
            0x0d02 => {
                matches!(args, [1, 1 | 2, 0 | 2 | 4 | 5 | 6 | 7, 0 | 1] | [0, 1 | 2, 0, 0 | 1])
            }
            0x0d07 => matches!(args, [1, 1, 0..=3] | [1, 2, 0..=2]),
            // The API only offers the tested range; wider encoding is needed to restore a captured setpoint.
            0x0d01 => matches!(args, [0 | 1, 1 | 2, 0..=55]),
            _ => false,
        };
        ensure!(allowed, "Command {command:04x} is blocked for Blade 18 controls");
        Ok(())
    }
    pub(crate) fn ensure_brightness_access(&self) -> Result<()> {
        if self.command_class == 0x03
            && self.command_id == 0x03
            && self.data_size == 3
            && self.args[..2] == [1, 5]
        {
            return Ok(());
        }
        self.ensure_diagnostic_query()
    }
    /// Only audited getter packets may pass through a diagnostic device.
    pub(crate) fn ensure_diagnostic_query(&self) -> Result<()> {
        let command = u16::from_be_bytes([self.command_class, self.command_id]);
        let args = &self.args[..usize::from(self.data_size).min(self.args.len())];
        let allowed = match command {
            0x0081 => args == [0, 0],
            0x0d85 => args == [0, 1, 0, 0],
            // Vendor model-711 middleware: fixed-shape capability getters only.
            0x0d80 => args == [0; 80],
            0x0d83 => args == [0; 12],
            0x0d8a | 0x0d8b => args == [0, 0],
            0x0d89 => args == [0, 0],
            0x0d8e | 0x0f98 => args == [0],
            // Model-711 Chroma inventory and underside LED (Blade2 getter uses class 0).
            0x0f80 => args == [0; 80],
            0x0f81 => {
                args.len() == 80 && matches!(args[0], 4 | 5) && args[1..].iter().all(|v| *v == 0)
            }
            0x0f82 => {
                args.len() == 80
                    && args[0] == 1
                    && matches!(args[1], 4 | 5)
                    && args[2..].iter().all(|v| *v == 0)
            }
            0x0f84 => matches!(args, [1, 4 | 5, 0]),
            0x0d90 => args == [0, 0, 0],
            0x0d8f => args == [0],
            0x0383 => matches!(args, [1, 5 | 38, 0]),
            0x0380 => args == [1, 4, 0] || args == [0, 38, 0],
            0x0382 => args == [1, 4, 0] || args == [1, 5, 0],
            0x0381 => {
                args == [1, 4, 0, 0, 0] || args == [1, 5, 0, 0, 0] || args == [0, 38, 0, 0, 0]
            }
            0x0084 => args == [0, 0],
            0x0792 | 0x078f => args == [0],
            // Selector 4 is included only as a getter after the device's ID list
            // explicitly advertised it. No fourth-selector setters are allowed.
            0x0d82 => matches!(args, [0, 1..=4, 0, 0]),
            0x0d81 | 0x0d88 => matches!(args, [0, 1..=4, 0]),
            0x0d87 => args == [0, 1, 0] || args == [0, 2, 0],
            _ => false,
        };
        ensure!(allowed, "Command {command:04x} is blocked in diagnostic mode");
        Ok(())
    }

    pub fn new(command: u16, args: &[u8]) -> Packet {
        let mut args_buffer = [0x00; 80];
        args_buffer[..args.len()].copy_from_slice(args);

        Packet {
            status: CommandStatus::New as u8,
            id: rand::thread_rng().gen(),
            remaining_packets: 0x0000,
            protocol_type: 0x00,
            data_size: args.len() as u8,
            command_class: (command >> 8) as u8,
            command_id: (command & 0xff) as u8,
            args: args_buffer,
            crc: 0x00,
            reserved: 0x00,
        }
    }

    pub fn set_args(&mut self, args: &[u8]) {
        self.args[..args.len()].copy_from_slice(args)
    }

    pub fn get_args(&self) -> &[u8] {
        &self.args
    }

    pub fn ensure_matches_report(&self, report: &Packet) -> Result<()> {
        ensure!(
            (report.command_class, report.command_id, report.id)
                == (self.command_class, self.command_id, self.id),
            "Response does not match the report"
        );

        ensure!(
            self.remaining_packets == report.remaining_packets
            || (self.command_class, self.command_id) == (0x07, 0x92) /* 0x0792 (bho) has special handling */
            || (self.command_class, self.command_id) == (0x07, 0x8f) /* 0x078f max fan speed mode has special handling */
            // Observed Blade 18 temperature response variant: wire bytes 00 03
            // in the nominal packet-count field (little-endian decoded as 0300).
            // Keep the exception specific to this command and payload shape.
            || ((self.command_class, self.command_id) == (0x0d, 0x85)
                && report.remaining_packets == 0
                && self.remaining_packets == 0x0300
                && self.data_size == 4
                && self.args[0] == 2)
            // Additional fixed getter variants observed on this Blade 18.
            // Keep command, request/response length and header value scoped;
            // this does not authorize queries to any extra fan selectors.
            || (report.remaining_packets == 0 && report.data_size == self.data_size
                && match (self.command_class, self.command_id, self.data_size, self.remaining_packets) {
                    (0x0d, 0x80, 80, 0x0500) => self.args[..5] == [4, 1, 2, 3, 4],
                    (0x0d, 0x83, 12, 0x0c00) => self.args[0] == 0,
                    (0x0d, 0x8f, 1, 0x0100) => self.args[0] == 1,
                    (0x0d, 0x8e, 1, 0x0100) => self.args[0] <= 1,
                    _ => false,
                }),
            "Response packet count {} differs from request {}; status {:02x}, data length {}, first arguments {:?}",
            self.remaining_packets, report.remaining_packets, self.status, self.data_size, &self.args[..8]
        );

        ensure!(self.status != CommandStatus::NotSupported as u8, "Command not supported");

        ensure!(
            self.status == CommandStatus::Successful as u8,
            "Command failed with unknown status: {:02X?}",
            self.status
        );

        Ok(())
    }
}

impl From<&Packet> for Vec<u8> {
    fn from(packet: &Packet) -> Vec<u8> {
        bincode::serialize(packet).unwrap()
    }
}

impl TryFrom<&[u8]> for Packet {
    type Error = anyhow::Error;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        ensure!(data.len() == std::mem::size_of::<Packet>(), "Invalid raw data size");

        Ok(bincode::deserialize::<Packet>(data)?)
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::Packet;
    #[test]
    fn static_keyboard_setter_cannot_write_other_regions_effects_or_frames() {
        let valid = [1, 5, 1, 0, 1, 1, 32, 64, 255];
        assert!(Packet::new(0x0f02, &valid).ensure_blade18_access().is_ok());
        assert!(Packet::new(0x0f02, &valid).ensure_diagnostic_query().is_err());
        for field in 0..6 {
            for value in 0..=255 {
                let mut args = valid;
                args[field] = value;
                assert_eq!(
                    Packet::new(0x0f02, &args).ensure_blade18_access().is_ok(),
                    value == valid[field] || (field == 2 && value == 2)
                );
            }
        }
        assert!(Packet::new(0x0f02, &valid[..8]).ensure_blade18_access().is_err());
        assert!(Packet::new(0x0f02, &[1, 5, 1, 0, 1, 1, 32, 64, 255, 0])
            .ensure_blade18_access()
            .is_err());
    }
    #[test]
    fn lighting_switches_cannot_enable_other_profiles_regions_or_effect_writes() {
        for profile in 0..=2 {
            for value in 0..=2 {
                let startup = Packet::new(0x0f18, &[profile, value]);
                assert_eq!(startup.ensure_blade18_access().is_ok(), profile == 0 && value <= 1);
                assert!(startup.ensure_diagnostic_query().is_err());
                let underside = Packet::new(0x0300, &[profile, 38, value]);
                assert_eq!(underside.ensure_blade18_access().is_ok(), profile == 1 && value <= 1);
                assert!(underside.ensure_diagnostic_query().is_err());
            }
        }
        for command in [0x0f02, 0x0f03, 0x0f04, 0x0f05, 0x0f06] {
            assert!(Packet::new(command, &[0; 80]).ensure_blade18_access().is_err());
        }
        for region in [0, 1, 3, 4, 5, 6, 38, 255] {
            let mut args = [0; 80];
            args[0] = region;
            assert_eq!(
                Packet::new(0x0f81, &args).ensure_diagnostic_query().is_ok(),
                matches!(region, 4 | 5)
            );
            args[0] = 1;
            args[1] = region;
            assert_eq!(
                Packet::new(0x0f82, &args).ensure_diagnostic_query().is_ok(),
                matches!(region, 4 | 5)
            );
            args[79] = 1;
            assert!(Packet::new(0x0f82, &args).ensure_diagnostic_query().is_err());
        }
    }
    #[test]
    fn standard_framing_encodes_known_checksums_without_changing_commands() {
        for (command, args, expected) in
            [(0x0d85, vec![0, 1, 0, 0], 0x8d), (0x0d01, vec![1, 1, 45], 0x22)]
        {
            let packet = Packet::new(command, &args).standard_framing();
            let bytes = Vec::<u8>::from(&packet);
            assert_eq!(bytes.len(), 90);
            assert_eq!(bytes[1], 0x1f);
            assert_eq!(bytes[88], expected);
            assert_eq!(&bytes[8..8 + args.len()], args.as_slice());
            assert_eq!(bytes[2..89].iter().fold(0, |crc, byte| crc ^ byte), 0);
        }
    }
    #[test]
    fn fan_lab_limits_and_normal_access_remain_separate() {
        for zone in 0..=4 {
            for mode in 0..=7 {
                for manual in 0..=2 {
                    let packet = Packet::new(0x0d02, &[0, zone, mode, manual]);
                    assert_eq!(
                        packet.ensure_fan_lab_access().is_ok(),
                        matches!(zone, 1 | 2) && mode == 0 && manual <= 1
                    );
                    assert_eq!(
                        packet.ensure_blade18_access().is_ok(),
                        matches!(zone, 1 | 2) && mode == 0 && manual <= 1
                    );
                }
            }
        }
        // Exhaust the one-byte enable/selector/RPM domain, including malformed lengths.
        for enable in 0..=2 {
            for zone in 0..=4 {
                for rpm in 0..=255 {
                    let packet = Packet::new(0x0d01, &[enable, zone, rpm]);
                    let allowed = enable <= 1
                        && match zone {
                            1 | 2 => rpm <= 55,
                            3 => rpm <= 35,
                            _ => false,
                        };
                    assert_eq!(packet.ensure_fan_lab_access().is_ok(), allowed);
                    assert_eq!(
                        packet.ensure_blade18_access().is_ok(),
                        enable <= 1 && matches!(zone, 1 | 2) && rpm <= 55
                    );
                    assert!(packet.ensure_diagnostic_query().is_err());
                }
            }
        }
        for args in [vec![], vec![1, 3], vec![1, 3, 35, 0]] {
            assert!(Packet::new(0x0d01, &args).ensure_fan_lab_access().is_err());
        }
        assert!(Packet::new(0x0d07, &[1, 1, 4]).ensure_fan_lab_access().is_err());
    }
    #[test]
    fn blade18_access_blocks_undervolt_unknown_modes_and_unbounded_fans() {
        for (cmd, args) in [
            (0x0d07, vec![1, 1, 4]),
            (0x0d02, vec![1, 1, 3, 0]),
            (0x0d01, vec![0, 1, 56]),
            (0x0004, vec![3, 0]),
            (0x070f, vec![2]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_blade18_access().is_err());
        }
    }

    #[test]
    fn brightness_access_does_not_enable_other_setters() {
        assert!(Packet::new(0x0303, &[1, 5, 160]).ensure_brightness_access().is_ok());
        for (cmd, args) in [
            (0x0303, vec![1, 4, 160]),
            (0x0303, vec![1, 5]),
            (0x0300, vec![1, 4, 1]),
            (0x0d02, vec![1, 1, 0, 0]),
            (0x0712, vec![1]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_brightness_access().is_err());
        }
    }

    #[test]
    fn diagnostics_reject_setters_and_unreviewed_arguments() {
        for (cmd, args) in [
            (0x0303, vec![1, 5, 50]),
            (0x0d02, vec![1, 1, 0, 0]),
            (0x0712, vec![1]),
            (0x0d01, vec![0, 1, 20]),
            (0x0383, vec![1, 4, 0]),
            (0xffff, vec![]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_diagnostic_query().is_err());
        }
    }

    #[test]
    fn diagnostics_allow_reviewed_getters() {
        for (cmd, args) in [
            (0x0383, vec![1, 5, 0]),
            (0x0792, vec![0]),
            (0x0d82, vec![0, 2, 0, 0]),
            (0x0d88, vec![0, 1, 0]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_diagnostic_query().is_ok());
        }
    }
    #[test]
    fn thermal_capability_queries_cannot_become_setters_or_selector_scans() {
        for (command, size) in [
            (0x0d80, 80),
            (0x0d83, 12),
            (0x0d8a, 2),
            (0x0d8b, 2),
            (0x0d8f, 1),
            (0x0d89, 2),
            (0x0d8e, 1),
            (0x0d90, 3),
            (0x0f98, 1),
        ] {
            assert!(Packet::new(command, &vec![0; size]).ensure_diagnostic_query().is_ok());
            assert!(Packet::new(command & 0xff7f, &vec![0; size]).ensure_blade18_access().is_err());
            let mut unknown = vec![0; size];
            unknown[0] = 1;
            assert!(Packet::new(command, &unknown).ensure_diagnostic_query().is_err());
        }
    }

    #[test]
    fn lighting_research_allows_only_source_backed_getters() {
        assert!(Packet::new(0x0381, &[1, 4, 0, 0, 0]).ensure_diagnostic_query().is_ok());
        assert!(Packet::new(0x0382, &[1, 5, 0]).ensure_diagnostic_query().is_ok());
        for (cmd, args) in [
            (0x0301, vec![1, 4, 255, 0, 0]),
            (0x0381, vec![1, 6, 0, 0, 0]),
            (0x0381, vec![1, 4, 0]),
            (0x0382, vec![1, 9, 0]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_diagnostic_query().is_err());
        }
    }

    #[test]
    fn sensor_discovery_does_not_enable_third_fan_writes_or_arbitrary_scans() {
        for (cmd, args) in [
            (0x0d88, vec![0, 3, 0]),
            (0x0d81, vec![0, 3, 0]),
            (0x0d88, vec![0, 4, 0]),
            (0x0d82, vec![0, 4, 0, 0]),
            (0x0d85, vec![0, 1, 0, 0]),
            (0x0081, vec![0, 0]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_diagnostic_query().is_ok());
        }
        for (cmd, args) in [
            (0x0d01, vec![0, 3, 40]),
            (0x0d02, vec![1, 3, 0, 1]),
            (0x0d88, vec![0, 5, 0]),
            (0x0d01, vec![1, 4, 30]),
            (0x0d02, vec![0, 4, 0, 1]),
            (0x0d85, vec![0, 4, 0, 0]),
        ] {
            assert!(Packet::new(cmd, &args).ensure_blade18_access().is_err());
        }
    }

    #[test]
    fn temperature_header_variant_requires_matching_command_and_payload() {
        let report = Packet::new(0x0d85, &[0, 1, 0, 0]);
        let mut response = Packet::new(0x0d85, &[2, 69, 45, 0]);
        response.id = report.id;
        response.status = 2;
        response.remaining_packets = 0x0300;
        assert!(response.ensure_matches_report(&report).is_ok());
        response.remaining_packets = 0x0400;
        assert!(response.ensure_matches_report(&report).is_err());
        response.remaining_packets = 0x0300;
        response.args[0] = 3;
        assert!(response.ensure_matches_report(&report).is_err());
        response.args[0] = 2;
        response.command_id = 0x88;
        assert!(response.ensure_matches_report(&report).is_err());
    }

    #[test]
    fn capability_header_variants_are_scoped_to_observed_shapes() {
        for (command, size, header, prefix) in [
            (0x0d80, 80, 0x0500, vec![4, 1, 2, 3, 4]),
            (0x0d83, 12, 0x0c00, vec![0]),
            (0x0d8f, 1, 0x0100, vec![1]),
            (0x0d8e, 1, 0x0100, vec![0]),
        ] {
            let request = Packet::new(command, &vec![0; size]);
            let mut response = Packet::new(command, &vec![0; size]);
            response.args[..prefix.len()].copy_from_slice(&prefix);
            response.id = request.id;
            response.status = 2;
            response.remaining_packets = header;
            assert!(response.ensure_matches_report(&request).is_ok());
            response.remaining_packets += 0x100;
            assert!(response.ensure_matches_report(&request).is_err());
            response.remaining_packets = header;
            response.data_size -= 1;
            assert!(response.ensure_matches_report(&request).is_err());
        }
    }
}
