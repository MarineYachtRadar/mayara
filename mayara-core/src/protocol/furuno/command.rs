//! Furuno radar command formatting
//!
//! Pure functions for building Furuno protocol command strings.
//! No I/O operations - just returns formatted strings ready to send.

use std::fmt::Write;

// =============================================================================
// Command Mode
// =============================================================================

/// Command mode prefix character
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    /// Set a value (prefix 'S')
    Set,
    /// Request current value (prefix 'R')
    Request,
    /// New/response (prefix 'N')
    New,
}

impl CommandMode {
    /// Get the character prefix for this command mode
    pub fn as_char(self) -> char {
        match self {
            CommandMode::Set => 'S',
            CommandMode::Request => 'R',
            CommandMode::New => 'N',
        }
    }
}

// =============================================================================
// Command IDs
// =============================================================================

/// Furuno command IDs (hex values used in protocol)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommandId {
    Connect = 0x60,
    Range = 0x62,
    Gain = 0x63,
    Sea = 0x64,
    Rain = 0x65,
    CustomPictureAll = 0x66,
    Status = 0x69,
    BlindSector = 0x77,
    AntennaHeight = 0x84,
    AliveCheck = 0xE3,
}

impl CommandId {
    /// Get the hex value for this command
    pub fn as_hex(self) -> u8 {
        self as u8
    }
}

// =============================================================================
// Login Protocol
// =============================================================================

/// Login message sent to port 10000 to get dynamic command port
/// From fnet.dll function "login_via_copyright"
pub const LOGIN_MESSAGE: [u8; 56] = [
    0x08, 0x01, 0x00, 0x38, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    // "COPYRIGHT (C) 2001 FURUNO ELECTRIC CO.,LTD. "
    0x43, 0x4f, 0x50, 0x59, 0x52, 0x49, 0x47, 0x48, 0x54, 0x20, 0x28, 0x43,
    0x29, 0x20, 0x32, 0x30, 0x30, 0x31, 0x20, 0x46, 0x55, 0x52, 0x55, 0x4e,
    0x4f, 0x20, 0x45, 0x4c, 0x45, 0x43, 0x54, 0x52, 0x49, 0x43, 0x20, 0x43,
    0x4f, 0x2e, 0x2c, 0x4c, 0x54, 0x44, 0x2e, 0x20,
];

/// Expected header in login response (8 bytes)
pub const LOGIN_RESPONSE_HEADER: [u8; 8] = [0x09, 0x01, 0x00, 0x0c, 0x01, 0x00, 0x00, 0x00];

/// Parse login response to extract the dynamic command port
///
/// The radar responds with 12 bytes total:
/// - Bytes 0-7: Header (LOGIN_RESPONSE_HEADER)
/// - Bytes 8-9: Port offset (big-endian)
/// - Bytes 10-11: Unknown
///
/// Returns the port number (BASE_PORT + offset) if valid
pub fn parse_login_response(data: &[u8]) -> Option<u16> {
    if data.len() < 12 {
        return None;
    }
    if data[0..8] != LOGIN_RESPONSE_HEADER {
        return None;
    }
    // Port offset is in bytes 8-9, big-endian
    let port_offset = ((data[8] as u16) << 8) | (data[9] as u16);
    Some(super::BASE_PORT + port_offset)
}

// =============================================================================
// Command Formatting Functions
// =============================================================================

/// Format a generic Furuno command
///
/// # Arguments
/// * `mode` - Command mode (Set, Request, New)
/// * `id` - Command ID
/// * `args` - Command arguments
///
/// # Returns
/// Formatted command string with \r\n terminator
///
/// # Example
/// ```
/// use mayara_core::protocol::furuno::command::{format_command, CommandMode, CommandId};
/// let cmd = format_command(CommandMode::Set, CommandId::Status, &[2, 0, 0, 60, 300, 0]);
/// assert_eq!(cmd, "$S69,2,0,0,60,300,0\r\n");
/// ```
pub fn format_command(mode: CommandMode, id: CommandId, args: &[i32]) -> String {
    let mut message = format!("${}{:X}", mode.as_char(), id.as_hex());
    for arg in args {
        let _ = write!(&mut message, ",{}", arg);
    }
    message.push_str("\r\n");
    message
}

/// Format status command (transmit/standby)
///
/// Command 0x69 controls radar power state:
/// - value=2: Transmit
/// - value=1: Standby
///
/// # Arguments
/// * `transmit` - true for transmit, false for standby
///
/// # Returns
/// Formatted command: `$S69,{1|2},0,0,60,300,0\r\n`
pub fn format_status_command(transmit: bool) -> String {
    let value = if transmit { 2 } else { 1 };
    // Args: status, 0, watchman_on_off, watchman_on_time, watchman_off_time, 0
    format_command(CommandMode::Set, CommandId::Status, &[value, 0, 0, 60, 300, 0])
}

/// Format range command
///
/// # Arguments
/// * `range_index` - Index into the radar's range table (0-23)
///
/// # Returns
/// Formatted command: `$S62,{index},0,0\r\n`
pub fn format_range_command(range_index: i32) -> String {
    format_command(CommandMode::Set, CommandId::Range, &[range_index, 0, 0])
}

/// Format gain command
///
/// # Arguments
/// * `value` - Gain value (0-100)
/// * `auto` - true for automatic gain control
///
/// # Returns
/// Formatted command: `$S63,0,{value},{auto},{value},0\r\n`
pub fn format_gain_command(value: i32, auto: bool) -> String {
    let auto_val = if auto { 1 } else { 0 };
    format_command(CommandMode::Set, CommandId::Gain, &[0, value, auto_val, value, 0])
}

/// Format sea clutter command
///
/// # Arguments
/// * `value` - Sea clutter value
///
/// # Returns
/// Formatted command: `$S64,{value}\r\n`
pub fn format_sea_command(value: i32) -> String {
    format_command(CommandMode::Set, CommandId::Sea, &[value])
}

/// Format rain clutter command
///
/// # Arguments
/// * `value` - Rain clutter value
///
/// # Returns
/// Formatted command: `$S65,0,{value}\r\n`
pub fn format_rain_command(value: i32) -> String {
    format_command(CommandMode::Set, CommandId::Rain, &[0, value])
}

/// Format keep-alive (alive check) command
///
/// Should be sent every 5 seconds to maintain connection
///
/// # Returns
/// Formatted command: `$RE3\r\n`
pub fn format_keepalive() -> String {
    format_command(CommandMode::Request, CommandId::AliveCheck, &[])
}

/// Format request for all picture settings
///
/// # Returns
/// Formatted command: `$R66\r\n`
pub fn format_request_picture_all() -> String {
    format_command(CommandMode::Request, CommandId::CustomPictureAll, &[])
}

// =============================================================================
// Response Parsing
// =============================================================================

/// Parse a Furuno response line
///
/// Response format: `${mode}{command_id},{args...}`
///
/// # Returns
/// Tuple of (CommandMode, command_id, Vec<args>) if valid
pub fn parse_response(line: &str) -> Option<(CommandMode, u8, Vec<i32>)> {
    let line = line.trim();
    if !line.starts_with('$') || line.len() < 3 {
        return None;
    }

    let mode = match line.chars().nth(1)? {
        'S' => CommandMode::Set,
        'R' => CommandMode::Request,
        'N' => CommandMode::New,
        _ => return None,
    };

    // Parse command ID (hex, 1-2 chars)
    let rest = &line[2..];
    let comma_pos = rest.find(',').unwrap_or(rest.len());
    let cmd_id = u8::from_str_radix(&rest[..comma_pos], 16).ok()?;

    // Parse arguments
    let mut args = Vec::new();
    if comma_pos < rest.len() {
        for arg in rest[comma_pos + 1..].split(',') {
            if let Ok(val) = arg.trim().parse::<i32>() {
                args.push(val);
            }
        }
    }

    Some((mode, cmd_id, args))
}

/// Parse status response to get current radar state
///
/// Response: `$N69,{status},0,...`
/// - status=1: Standby
/// - status=2: Transmit
///
/// # Returns
/// true if transmitting, false if standby, None if invalid
pub fn parse_status_response(line: &str) -> Option<bool> {
    let (mode, cmd_id, args) = parse_response(line)?;
    if mode != CommandMode::New || cmd_id != CommandId::Status.as_hex() {
        return None;
    }
    args.first().map(|&status| status == 2)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_status_transmit() {
        let cmd = format_status_command(true);
        assert_eq!(cmd, "$S69,2,0,0,60,300,0\r\n");
    }

    #[test]
    fn test_format_status_standby() {
        let cmd = format_status_command(false);
        assert_eq!(cmd, "$S69,1,0,0,60,300,0\r\n");
    }

    #[test]
    fn test_format_range() {
        let cmd = format_range_command(5);
        assert_eq!(cmd, "$S62,5,0,0\r\n");
    }

    #[test]
    fn test_format_gain_manual() {
        let cmd = format_gain_command(75, false);
        assert_eq!(cmd, "$S63,0,75,0,75,0\r\n");
    }

    #[test]
    fn test_format_gain_auto() {
        let cmd = format_gain_command(50, true);
        assert_eq!(cmd, "$S63,0,50,1,50,0\r\n");
    }

    #[test]
    fn test_format_keepalive() {
        let cmd = format_keepalive();
        assert_eq!(cmd, "$RE3\r\n");
    }

    #[test]
    fn test_parse_login_response() {
        // Simulated response with port offset 0x0001 = 1
        let response: [u8; 12] = [
            0x09, 0x01, 0x00, 0x0c, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x01, // Port offset = 1
            0x00, 0x00,
        ];
        let port = parse_login_response(&response);
        assert_eq!(port, Some(10001)); // BASE_PORT + 1
    }

    #[test]
    fn test_parse_response() {
        let (mode, cmd_id, args) = parse_response("$N69,2,0,0,60,300,0").unwrap();
        assert_eq!(mode, CommandMode::New);
        assert_eq!(cmd_id, 0x69);
        assert_eq!(args, vec![2, 0, 0, 60, 300, 0]);
    }

    #[test]
    fn test_parse_status_response() {
        assert_eq!(parse_status_response("$N69,2,0,0,60,300,0"), Some(true));
        assert_eq!(parse_status_response("$N69,1,0,0,60,300,0"), Some(false));
        assert_eq!(parse_status_response("$N62,5,0,0"), None); // Wrong command
    }
}
