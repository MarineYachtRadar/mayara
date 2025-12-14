//! Doppler State Machine for ARPA
//!
//! Tracks whether targets are approaching or receding based on Doppler radar data.

use serde::{Deserialize, Serialize};

/// Doppler states of a target.
///
/// Determines the search method for the target in the history array.
/// The target pixel must match the Doppler state to be considered part of the target.
///
/// # Bit interpretation for pixel matching
///
/// - bit0: Above threshold (TARGET)
/// - bit2: APPROACHING
/// - bit3: RECEDING
///
/// | State          | bit0 | bit2 | bit3 |
/// |----------------|------|------|------|
/// | Any            | 1    | x    | x    |
/// | NoDoppler      | 1    | 0    | 0    |
/// | Approaching    | 1    | 1    | 0    |
/// | Receding       | 1    | 0    | 1    |
/// | AnyDoppler     | 1    | 1/0  | 0/1  |
/// | NotReceding    | 1    | x    | 0    |
/// | NotApproaching | 1    | 0    | x    |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DopplerState {
    /// Any target above threshold (non-Doppler target)
    Any,
    /// A target without any Doppler bit set
    NoDoppler,
    /// Doppler approaching target
    Approaching,
    /// Doppler receding target
    Receding,
    /// Approaching or Receding (either Doppler bit set)
    AnyDoppler,
    /// Not receding (NoDoppler or Approaching)
    NotReceding,
    /// Not approaching (NoDoppler or Receding)
    NotApproaching,
    /// Will also check pixels that have been cleared (backup bit)
    AnyPlus,
}

impl Default for DopplerState {
    fn default() -> Self {
        DopplerState::Any
    }
}

impl DopplerState {
    /// Determine state transition based on pixel counts within a target contour.
    ///
    /// # Arguments
    /// * `total_pix` - Total pixels in the target
    /// * `approaching_pix` - Pixels with Doppler approaching bit
    /// * `receding_pix` - Pixels with Doppler receding bit
    ///
    /// # Returns
    /// New Doppler state based on pixel composition
    pub fn transition(
        &self,
        total_pix: u32,
        approaching_pix: u32,
        receding_pix: u32,
    ) -> DopplerState {
        // Threshold: 85% of pixels must be Doppler to classify
        let check_to_doppler = (total_pix as f64 * 0.85) as u32;
        // Threshold for transition away: 80% of non-X pixels
        let check_not_approaching = ((total_pix - approaching_pix) as f64 * 0.80) as u32;
        let check_not_receding = ((total_pix - receding_pix) as f64 * 0.80) as u32;

        match self {
            DopplerState::AnyDoppler | DopplerState::Any => {
                // Try to classify as Approaching or Receding
                if approaching_pix > receding_pix && approaching_pix > check_to_doppler {
                    DopplerState::Approaching
                } else if receding_pix > approaching_pix && receding_pix > check_to_doppler {
                    DopplerState::Receding
                } else if *self == DopplerState::AnyDoppler {
                    // AnyDoppler falls back to Any if can't classify
                    DopplerState::Any
                } else {
                    // Any stays Any
                    *self
                }
            }

            DopplerState::Receding => {
                // Transition to Any if not enough receding pixels
                if receding_pix < check_not_approaching {
                    DopplerState::Any
                } else {
                    *self
                }
            }

            DopplerState::Approaching => {
                // Transition to Any if not enough approaching pixels
                if approaching_pix < check_not_receding {
                    DopplerState::Any
                } else {
                    *self
                }
            }

            // Other states don't transition
            _ => *self,
        }
    }

    /// Check if this state matches the given pixel flags
    ///
    /// # Arguments
    /// * `is_target` - Pixel is above threshold
    /// * `is_backup` - Pixel has backup bit (was target in previous scan)
    /// * `is_approaching` - Pixel has Doppler approaching bit
    /// * `is_receding` - Pixel has Doppler receding bit
    pub fn matches_pixel(
        &self,
        is_target: bool,
        is_backup: bool,
        is_approaching: bool,
        is_receding: bool,
    ) -> bool {
        match self {
            DopplerState::Any => is_target,
            DopplerState::NoDoppler => is_target && !is_approaching && !is_receding,
            DopplerState::Approaching => is_approaching,
            DopplerState::Receding => is_receding,
            DopplerState::AnyDoppler => is_approaching || is_receding,
            DopplerState::NotReceding => is_target && !is_receding,
            DopplerState::NotApproaching => is_target && !is_approaching,
            DopplerState::AnyPlus => is_backup,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_to_approaching() {
        let state = DopplerState::Any;
        // 90 of 100 pixels are approaching
        let new_state = state.transition(100, 90, 5);
        assert_eq!(new_state, DopplerState::Approaching);
    }

    #[test]
    fn test_transition_to_receding() {
        let state = DopplerState::AnyDoppler;
        // 90 of 100 pixels are receding
        let new_state = state.transition(100, 5, 90);
        assert_eq!(new_state, DopplerState::Receding);
    }

    #[test]
    fn test_no_transition_mixed() {
        let state = DopplerState::Any;
        // Mixed pixels, stays Any
        let new_state = state.transition(100, 40, 40);
        assert_eq!(new_state, DopplerState::Any);
    }

    #[test]
    fn test_receding_falls_back_to_any() {
        let state = DopplerState::Receding;
        // Only 10 receding pixels out of 100
        let new_state = state.transition(100, 5, 10);
        assert_eq!(new_state, DopplerState::Any);
    }

    #[test]
    fn test_matches_pixel() {
        assert!(DopplerState::Any.matches_pixel(true, true, false, false));
        assert!(!DopplerState::Any.matches_pixel(false, true, false, false));

        assert!(DopplerState::Approaching.matches_pixel(true, true, true, false));
        assert!(!DopplerState::Approaching.matches_pixel(true, true, false, true));

        assert!(DopplerState::AnyDoppler.matches_pixel(true, true, true, false));
        assert!(DopplerState::AnyDoppler.matches_pixel(true, true, false, true));
        assert!(!DopplerState::AnyDoppler.matches_pixel(true, true, false, false));
    }
}
