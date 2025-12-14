use std::str::FromStr;

use crate::radar::{RadarError, Status};
use crate::settings::{ControlValue, SharedControls};

use super::Command;

fn one_byte_command(cmd: &mut Vec<u8>, lead: &[u8], value: u8) {
    cmd.extend_from_slice(lead);
    cmd.extend_from_slice(&[0x28, 0x00, 0x00, value, 0x00, 0x00]);
}

fn two_byte_command(cmd: &mut Vec<u8>, lead: &[u8], value: u16) {
    cmd.extend_from_slice(lead);
    cmd.extend_from_slice(&[0x28, 0x00]);
    cmd.extend_from_slice(&value.to_le_bytes());
    cmd.extend_from_slice(&[0x00, 0x00]);
}

pub async fn set_control(
    command: &mut Command,
    cv: &ControlValue,
    value: f32,
    controls: &SharedControls,
) -> Result<(), RadarError> {
    let auto: u8 = if cv.auto.unwrap_or(false) { 1 } else { 0 };
    let enabled: u8 = if cv.enabled.unwrap_or(false) { 1 } else { 0 };
    let v = Command::scale_100_to_byte(value); // todo! use transform values

    let mut cmd = Vec::with_capacity(6);

    match cv.id.as_str() {
        "power" => {
            // Use core definition's enum values for case-insensitive lookup
            let value = if let Some(control) = controls.get("power") {
                // Look up the index: "transmit" -> 2, "standby" -> 1, etc.
                let index = control.enum_value_to_index(&cv.value).unwrap_or(1); // Default to standby
                if index == 2 { 1u8 } else { 0u8 } // transmit is index 2, wire value 1
            } else {
                // Fallback to old method if control not found
                match Status::from_str(&cv.value).unwrap_or(Status::Standby) {
                    Status::Transmit => 1,
                    _ => 0,
                }
            };
            cmd.extend_from_slice(&[0x01, 0x80, 0x01, 0x00, value, 0x00, 0x00, 0x00]);
        }

        "range" => {
            let value = value as i32;
            let ranges = &command.info.ranges;
            let index = if value < ranges.len() as i32 {
                value as u8
            } else {
                let mut i = 0;
                for r in ranges.all.iter() {
                    if r.distance() >= value {
                        break;
                    }
                    i += 1;
                }
                i
            };
            log::trace!("range {value} -> {index}");
            one_byte_command(&mut cmd, &[0x01, 0x01], index);
        }
        "gain" => {
            one_byte_command(&mut cmd, &[0x01, 0x03], auto);
            if auto == 0 {
                command.send(&cmd).await?;
                cmd.clear();
                one_byte_command(&mut cmd, &[0x02, 0x83], v);
            }
        }
        "colorGain" => {
            one_byte_command(&mut cmd, &[0x03, 0x03], auto);
            if auto == 0 {
                command.send(&cmd).await?;
                cmd.clear();
                one_byte_command(&mut cmd, &[0x04, 0x03], v);
            }
        }
        "sea" => {
            one_byte_command(&mut cmd, &[0x05, 0x03], auto);
            if auto == 0 {
                command.send(&cmd).await?;
                cmd.clear();
                one_byte_command(&mut cmd, &[0x06, 0x03], v);
            }
        }
        "rain" => {
            one_byte_command(&mut cmd, &[0x0b, 0x03], enabled);
            if enabled > 0 {
                command.send(&cmd).await?;
                cmd.clear();
                one_byte_command(&mut cmd, &[0x0c, 0x03], v);
            }
        }
        "targetExpansion" => {
            one_byte_command(&mut cmd, &[0x0f, 0x03], v);
        }
        "interferenceRejection" => {
            one_byte_command(&mut cmd, &[0x11, 0x03], v);
        }
        "mode" => {
            one_byte_command(&mut cmd, &[0x14, 0x03], v);
        }
        "bearingAlignment" => {
            let deci_value = (value * 10.0) as i16;
            two_byte_command(&mut cmd, &[0x01, 0x04], deci_value as u16);
        }
        "mainBangSuppression" => {
            todo!("Not implemented yet");
        }
        "noTransmitStart1" => {
            todo!("Not implemented yet");
        }
        "noTransmitStart2" => {
            todo!("Not implemented yet");
        }
        "noTransmitEnd1" => {
            todo!("Not implemented yet");
        }
        "noTransmitEnd2" => {
            todo!("Not implemented yet");
        }

        // Non-hardware settings
        _ => return Err(RadarError::CannotSetControlType(cv.id.clone())),
    };

    log::info!("{}: Send command {:02X?}", command.info.key(), cmd);
    command.send(&cmd).await?;

    Ok(())
}
