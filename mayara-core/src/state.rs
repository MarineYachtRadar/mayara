//! Radar State Tracking
//!
//! This module provides types for tracking the current state of radar controls.
//! State is updated by parsing responses from the radar and can be serialized
//! for the REST API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::protocol::furuno::command::{
    parse_gain_response, parse_rain_response, parse_range_response, parse_sea_response,
    parse_status_response, range_index_to_meters, ControlValue as ParsedControlValue,
};

/// Power state of the radar
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerState {
    Off,
    Standby,
    Transmit,
    Warming,
}

impl Default for PowerState {
    fn default() -> Self {
        PowerState::Off
    }
}

/// Control value with auto/manual mode (API format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlValueState {
    pub mode: String, // "auto" or "manual"
    pub value: i32,
}

impl Default for ControlValueState {
    fn default() -> Self {
        ControlValueState {
            mode: "auto".to_string(),
            value: 50,
        }
    }
}

impl From<ParsedControlValue> for ControlValueState {
    fn from(cv: ParsedControlValue) -> Self {
        ControlValueState {
            mode: if cv.auto { "auto" } else { "manual" }.to_string(),
            value: cv.value,
        }
    }
}

/// Complete radar state
///
/// Contains current values for all readable controls.
/// Updated by parsing $N responses from the radar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarState {
    /// Current power state
    pub power: PowerState,

    /// Current range in meters
    pub range: u32,

    /// Gain control state
    pub gain: ControlValueState,

    /// Sea clutter control state
    pub sea: ControlValueState,

    /// Rain clutter control state
    pub rain: ControlValueState,

    /// Timestamp of last update (milliseconds since epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

impl RadarState {
    /// Create a new radar state with default values
    pub fn new() -> Self {
        RadarState::default()
    }

    /// Update state by parsing a response line from the radar
    ///
    /// Returns true if the state was updated, false if the line wasn't recognized
    pub fn update_from_response(&mut self, line: &str) -> bool {
        // Try status response ($N69)
        if let Some(transmitting) = parse_status_response(line) {
            self.power = if transmitting {
                PowerState::Transmit
            } else {
                PowerState::Standby
            };
            return true;
        }

        // Try gain response ($N63)
        if let Some(cv) = parse_gain_response(line) {
            self.gain = cv.into();
            return true;
        }

        // Try sea response ($N64)
        if let Some(cv) = parse_sea_response(line) {
            self.sea = cv.into();
            return true;
        }

        // Try rain response ($N65)
        if let Some(cv) = parse_rain_response(line) {
            self.rain = cv.into();
            return true;
        }

        // Try range response ($N62)
        if let Some(range_index) = parse_range_response(line) {
            if let Some(meters) = range_index_to_meters(range_index) {
                self.range = meters as u32;
                return true;
            }
        }

        false
    }

    /// Convert to HashMap for API response
    ///
    /// Returns control values in the format expected by the /state endpoint
    pub fn to_controls_map(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();

        // Power state
        let power_str = match self.power {
            PowerState::Off => "off",
            PowerState::Standby => "standby",
            PowerState::Transmit => "transmit",
            PowerState::Warming => "warming",
        };
        map.insert("power".to_string(), serde_json::json!(power_str));

        // Range
        map.insert("range".to_string(), serde_json::json!(self.range));

        // Gain
        map.insert(
            "gain".to_string(),
            serde_json::json!({
                "mode": self.gain.mode,
                "value": self.gain.value
            }),
        );

        // Sea
        map.insert(
            "sea".to_string(),
            serde_json::json!({
                "mode": self.sea.mode,
                "value": self.sea.value
            }),
        );

        // Rain
        map.insert(
            "rain".to_string(),
            serde_json::json!({
                "mode": self.rain.mode,
                "value": self.rain.value
            }),
        );

        map
    }
}

/// Generate all request commands to query current state
///
/// Returns a vector of command strings that should be sent to the radar
/// to query all readable control values.
pub fn generate_state_requests() -> Vec<String> {
    use crate::protocol::furuno::command::{
        format_request_gain, format_request_rain, format_request_range, format_request_sea,
        format_request_status,
    };

    vec![
        format_request_status(),
        format_request_range(),
        format_request_gain(),
        format_request_sea(),
        format_request_rain(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_from_status_response() {
        let mut state = RadarState::new();

        // Transmit
        assert!(state.update_from_response("$N69,2,0,0,60,300,0"));
        assert_eq!(state.power, PowerState::Transmit);

        // Standby
        assert!(state.update_from_response("$N69,1,0,0,60,300,0"));
        assert_eq!(state.power, PowerState::Standby);
    }

    #[test]
    fn test_update_from_gain_response() {
        let mut state = RadarState::new();

        // Manual mode, value 75
        assert!(state.update_from_response("$N63,0,75,0,80,0"));
        assert_eq!(state.gain.mode, "manual");
        assert_eq!(state.gain.value, 75);

        // Auto mode, value 50
        assert!(state.update_from_response("$N63,1,50,0,80,0"));
        assert_eq!(state.gain.mode, "auto");
        assert_eq!(state.gain.value, 50);
    }

    #[test]
    fn test_update_from_range_response() {
        let mut state = RadarState::new();

        // Range index 5 = 2778m (1.5nm)
        assert!(state.update_from_response("$N62,5,0,0"));
        assert_eq!(state.range, 2778);

        // Range index 4 = 1852m (1nm)
        assert!(state.update_from_response("$N62,4,0,0"));
        assert_eq!(state.range, 1852);
    }

    #[test]
    fn test_to_controls_map() {
        let mut state = RadarState::new();
        state.power = PowerState::Transmit;
        state.range = 5556;
        state.gain = ControlValueState {
            mode: "manual".to_string(),
            value: 60,
        };

        let map = state.to_controls_map();

        assert_eq!(map.get("power").unwrap(), "transmit");
        assert_eq!(map.get("range").unwrap(), 5556);

        let gain = map.get("gain").unwrap();
        assert_eq!(gain["mode"], "manual");
        assert_eq!(gain["value"], 60);
    }

    #[test]
    fn test_generate_state_requests() {
        let requests = generate_state_requests();

        assert_eq!(requests.len(), 5);
        assert!(requests.contains(&"$R69\r\n".to_string()));
        assert!(requests.contains(&"$R62\r\n".to_string()));
        assert!(requests.contains(&"$R63\r\n".to_string()));
        assert!(requests.contains(&"$R64\r\n".to_string()));
        assert!(requests.contains(&"$R65\r\n".to_string()));
    }
}
