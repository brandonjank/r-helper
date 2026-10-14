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
    pub(crate) fn ensure_blade18_access(&self) -> Result<()> {
        if self.ensure_brightness_access().is_ok() {
            return Ok(());
        }
        let command = u16::from_be_bytes([self.command_class, self.command_id]);
        let args = &self.args[..usize::from(self.data_size).min(80)];
        let allowed = match command {
            0x0300 => matches!(args, [1, 4, 0 | 1]),
            0x0302 => matches!(args, [1, 4, 0 | 2]),
            0x0712 => matches!(args, [0x50 | 0xd0]),
            0x0d02 => matches!(args, [1, 1 | 2, 0 | 2 | 4 | 5 | 6 | 7, 0 | 1]),
            0x0d07 => matches!(args, [1, 1, 0..=3] | [1, 2, 0..=2]),
            // The API only offers the tested range; wider encoding is needed to restore a captured setpoint.
            0x0d01 => matches!(args, [0, 1 | 2, 0..=55]),
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
            0x0383 => args == [1, 5, 0],
            0x0380 | 0x0382 => args == [1, 4, 0],
            0x0084 => args == [0, 0],
            0x0792 | 0x078f => args == [0],
            0x0d82 => args == [0, 1, 0, 0] || args == [0, 2, 0, 0],
            0x0d81 | 0x0d88 | 0x0d87 => args == [0, 1, 0] || args == [0, 2, 0],
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
            || (self.command_class, self.command_id) == (0x07, 0x8f), /* 0x078f max fan speed mode has special handling */
            "Response command does not match the report"
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
}
