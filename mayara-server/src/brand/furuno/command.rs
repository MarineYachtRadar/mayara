use enum_primitive_derive::Primitive;
use std::fmt::Write;
use std::str::FromStr;
use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::net::TcpStream;

use super::CommandMode;
use crate::radar::range::Ranges;
use crate::radar::{RadarError, RadarInfo, Status};
use crate::settings::{ControlValue, SharedControls};

// Import mayara-core format functions for consistent command formatting
use mayara_core::protocol::furuno::command::{
    format_keepalive, format_range_command, format_request_picture_all,
};
use mayara_core::protocol::furuno::dispatch::format_control_command;

#[derive(Primitive, PartialEq, Eq, Debug, Clone)]
pub(crate) enum CommandId {
    Connect = 0x60,
    DispMode = 0x61,
    Range = 0x62,
    Gain = 0x63,
    Sea = 0x64,
    Rain = 0x65,
    CustomPictureAll = 0x66,
    CustomPicture = 0x67,
    Status = 0x69,
    U6D = 0x6D,
    AntennaType = 0x6E,

    BlindSector = 0x77,

    Att = 0x80,
    MainBangSize = 0x83,
    AntennaHeight = 0x84,
    NearSTC = 0x85,
    MiddleSTC = 0x86,
    FarSTC = 0x87,
    AntennaRevolution = 0x89,
    AntennaSwitch = 0x8A,
    AntennaNo = 0x8D,
    OnTime = 0x8E,

    Modules = 0x96,

    Drift = 0x9E,
    ConningPosition = 0xAA,
    WakeUpCount = 0xAC,

    STCRange = 0xD2,
    CustomMemory = 0xD3,
    BuildUpTime = 0xD4,
    DisplayUnitInformation = 0xD5,
    CustomATFSettings = 0xE0,
    AliveCheck = 0xE3,
    ATFSettings = 0xEA,
    BearingResolutionSetting = 0xEE,
    AccuShip = 0xF0,
    RangeSelect = 0xFE,
}

pub struct Command {
    key: String,
    controls: SharedControls,
    ranges: Ranges,
}

impl Command {
    pub fn new(info: &RadarInfo) -> Self {
        Command {
            key: info.key(),
            controls: info.controls.clone(),
            ranges: info.ranges.clone(),
        }
    }

    pub fn set_ranges(&mut self, ranges: Ranges) {
        self.ranges = ranges;
    }

    pub async fn send(
        &mut self,
        writer: &mut WriteHalf<TcpStream>,
        cm: CommandMode,
        id: CommandId,
        args: &[i32],
    ) -> Result<(), RadarError> {
        self.send_with_commas(writer, cm, id, args, 0).await
    }

    pub async fn send_with_commas(
        &mut self,
        writer: &mut WriteHalf<TcpStream>,
        cm: CommandMode,
        id: CommandId,
        args: &[i32],
        commas: u32,
    ) -> Result<(), RadarError> {
        let mut message = format!("${}{:X}", cm.as_char(), id as u32);
        for arg in args {
            let _ = write!(&mut message, ",{}", arg);
        }
        for _ in 0..commas {
            message.push(',');
        }

        log::trace!("{}: sending {}", self.key, message);

        if commas == 0 {
            message.push('\r');
        }
        message.push('\n');

        let bytes = message.into_bytes();

        writer.write_all(&bytes).await.map_err(RadarError::Io)?;

        Ok(())
    }

    /// Send a pre-formatted command string (from mayara-core format functions)
    pub async fn send_formatted(
        &self,
        writer: &mut WriteHalf<TcpStream>,
        message: &str,
    ) -> Result<(), RadarError> {
        log::trace!("{}: sending {}", self.key, message.trim());
        writer
            .write_all(message.as_bytes())
            .await
            .map_err(RadarError::Io)?;
        Ok(())
    }

    fn get_angle_value(&self, id: &str) -> i32 {
        if let Some(control) = self.controls.get(id) {
            if let Some(value) = control.value {
                return value as i32;
            }
        }
        return 0;
    }

    fn fill_blind_sector(
        &mut self,
        sector1_start: Option<i32>,
        sector1_end: Option<i32>,
        sector2_start: Option<i32>,
        sector2_end: Option<i32>,
    ) -> Vec<i32> {
        let mut cmd = Vec::with_capacity(6);

        cmd.push(
            sector1_start.unwrap_or_else(|| self.get_angle_value("noTransmitStart1")),
        );
        cmd.push(sector1_end.unwrap_or_else(|| self.get_angle_value("noTransmitEnd1")));
        cmd.push(
            sector2_start.unwrap_or_else(|| self.get_angle_value("noTransmitStart2")),
        );
        cmd.push(sector2_end.unwrap_or_else(|| self.get_angle_value("noTransmitEnd2")));

        cmd
    }

    pub async fn set_control(
        &mut self,
        write: &mut WriteHalf<TcpStream>,
        cv: &ControlValue,
    ) -> Result<(), RadarError> {
        let auto = cv.auto.unwrap_or(false);

        log::trace!("set_control: {:?} = {}", cv.id, cv.value);

        // Handle power separately since it uses string values, not numeric
        if cv.id == "power" {
            // Use core definition's enum values for case-insensitive lookup
            let value = if let Some(control) = self.controls.get("power") {
                // Look up the index: "transmit" -> 2, "standby" -> 1, etc.
                let index = control.enum_value_to_index(&cv.value);
                log::info!(
                    "{}: power lookup '{}' -> index {:?}, has_core_def={}",
                    self.key,
                    cv.value,
                    index,
                    control.core_definition().is_some()
                );
                index.unwrap_or(1) as i32
            } else {
                log::warn!(
                    "{}: power control not found, using Status::from_str fallback",
                    self.key
                );
                // Fallback to old method if control not found
                if Status::from_str(&cv.value).unwrap_or(Status::Standby) == Status::Transmit {
                    2
                } else {
                    1
                }
            };

            if let Some(cmd) = format_control_command("power", value, false) {
                log::info!("{}: Send command {}", self.key, cmd.trim());
                self.send_formatted(write, &cmd).await?;
            }

            // Request updated picture settings
            self.send_formatted(write, &format_request_picture_all())
                .await?;
            return Ok(());
        }

        // For other controls, parse the value as a number
        let value = cv
            .value
            .parse::<f32>()
            .map_err(|_| RadarError::MissingValue(cv.id.clone()))? as i32;

        // Handle range specially - needs index conversion
        if cv.id == "range" {
            let range_index = if value < self.ranges.len() as i32 {
                value
            } else {
                let mut i = 0;
                for r in self.ranges.all.iter() {
                    if r.distance() >= value {
                        break;
                    }
                    i += 1;
                }
                i
            };
            let cmd = format_range_command(range_index);
            log::info!("{}: Send command {}", self.key, cmd.trim());
            self.send_formatted(write, &cmd).await?;
            self.send_formatted(write, &format_request_picture_all())
                .await?;
            return Ok(());
        }

        // Handle blind sector controls specially - they need to aggregate multiple values
        if cv.id.starts_with("noTransmit") {
            let (command_id, cmd) = match cv.id.as_str() {
                "noTransmitStart1" => (
                    CommandId::BlindSector,
                    self.fill_blind_sector(Some(value), None, None, None),
                ),
                "noTransmitEnd1" => (
                    CommandId::BlindSector,
                    self.fill_blind_sector(None, Some(value), None, None),
                ),
                "noTransmitStart2" => (
                    CommandId::BlindSector,
                    self.fill_blind_sector(None, None, Some(value), None),
                ),
                "noTransmitEnd2" => (
                    CommandId::BlindSector,
                    self.fill_blind_sector(None, None, None, Some(value)),
                ),
                _ => return Err(RadarError::CannotSetControlType(cv.id.clone())),
            };
            log::info!(
                "{}: Send command {:02X},{:?}",
                self.key,
                command_id.clone() as u32,
                cmd
            );
            self.send(write, CommandMode::Set, command_id, &cmd).await?;
            self.send_formatted(write, &format_request_picture_all())
                .await?;
            return Ok(());
        }

        // Use mayara-core dispatch for all other controls
        if let Some(cmd) = format_control_command(&cv.id, value, auto) {
            log::info!("{}: Send command {}", self.key, cmd.trim());
            self.send_formatted(write, &cmd).await?;
        } else {
            return Err(RadarError::CannotSetControlType(cv.id.clone()));
        }

        // Request updated picture settings
        self.send_formatted(write, &format_request_picture_all())
            .await?;
        Ok(())
    }

    pub(crate) async fn init(
        &mut self,
        writer: &mut WriteHalf<TcpStream>,
    ) -> Result<(), RadarError> {
        self.send(writer, CommandMode::Request, CommandId::Connect, &[0])
            .await?; // $R60,0,0,0,0,0,0,0, Furuno sends with just separated commas.

        self.send_with_commas(writer, CommandMode::Request, CommandId::Modules, &[], 7)
            .await?; // $R96,,,,,,,

        self.send(writer, CommandMode::Request, CommandId::Range, &[0, 0, 0])
            .await?; // $R62,0,0,0

        self.send(
            writer,
            CommandMode::Request,
            CommandId::CustomPictureAll,
            &[],
        )
        .await?; // $R66
        self.send(
            writer,
            CommandMode::Request,
            CommandId::Status,
            &[0, 0, 0, 0, 0, 0],
        )
        .await?; // $R66,0,0,0,0,0,0

        self.send(
            writer,
            CommandMode::Request,
            CommandId::AntennaType,
            &[0, 0, 0, 0, 0, 0],
        )
        .await?; // $R6E,0,0,0,0,0,0,0

        self.send(
            writer,
            CommandMode::Request,
            CommandId::BlindSector,
            &[0, 0, 0, 0, 0],
        )
        .await?; // $R77,0,0,0,0,0

        self.send(
            writer,
            CommandMode::Request,
            CommandId::MainBangSize,
            &[0, 0],
        )
        .await?; // $R83,0,0

        self.send(
            writer,
            CommandMode::Request,
            CommandId::AntennaHeight,
            &[0, 0],
        )
        .await?; // $R84,0,0

        self.send(writer, CommandMode::Request, CommandId::NearSTC, &[0])
            .await?; // $R85,0

        self.send(writer, CommandMode::Request, CommandId::MiddleSTC, &[0])
            .await?; // $R86,0

        self.send(writer, CommandMode::Request, CommandId::FarSTC, &[0])
            .await?; // $R87,0

        self.send(writer, CommandMode::Request, CommandId::OnTime, &[0, 0])
            .await?; // $R8E,0

        self.send(writer, CommandMode::Request, CommandId::WakeUpCount, &[0])
            .await?; // $RAC,0

        Ok(())
    }

    pub(super) async fn send_report_requests(
        &mut self,
        writer: &mut WriteHalf<TcpStream>,
    ) -> Result<(), RadarError> {
        log::debug!("{}: send_report_requests", self.key);

        // Use mayara-core format_keepalive for consistent keepalive format
        self.send_formatted(writer, &format_keepalive()).await?;
        Ok(())
    }
}
