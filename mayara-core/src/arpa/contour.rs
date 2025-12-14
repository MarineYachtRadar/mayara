//! Contour Detection for ARPA
//!
//! Provides contour representation and error types for target tracking.

use super::polar::Polar;

/// Minimum contour length to consider a valid target
pub const MIN_CONTOUR_LENGTH: usize = 6;

/// Maximum contour length (prevents runaway on large blobs)
pub const MAX_CONTOUR_LENGTH: usize = 2000;

/// Contour detection errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourError {
    /// Range value too high (beyond spoke length)
    RangeTooHigh,
    /// Range value too low (inside main bang)
    RangeTooLow,
    /// No echo at starting position
    NoEchoAtStart,
    /// Starting point is not on the contour edge
    StartPointNotOnContour,
    /// Contour is broken (couldn't complete)
    BrokenContour,
    /// No contour found at search position
    NoContourFound,
    /// Target was already found in this scan
    AlreadyFound,
    /// Target not found
    NotFound,
    /// Contour too long (possible noise or interference)
    ContourTooLong,
    /// Target marked as lost
    Lost,
    /// Weighted contour length check failed
    WeightedContourLengthTooHigh,
    /// Waiting for next refresh cycle
    WaitForRefresh,
}

impl std::fmt::Display for ContourError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContourError::RangeTooHigh => write!(f, "Range too high"),
            ContourError::RangeTooLow => write!(f, "Range too low"),
            ContourError::NoEchoAtStart => write!(f, "No echo at start position"),
            ContourError::StartPointNotOnContour => write!(f, "Start point not on contour"),
            ContourError::BrokenContour => write!(f, "Broken contour"),
            ContourError::NoContourFound => write!(f, "No contour found"),
            ContourError::AlreadyFound => write!(f, "Target already found"),
            ContourError::NotFound => write!(f, "Target not found"),
            ContourError::ContourTooLong => write!(f, "Contour too long"),
            ContourError::Lost => write!(f, "Target lost"),
            ContourError::WeightedContourLengthTooHigh => {
                write!(f, "Weighted contour length too high")
            }
            ContourError::WaitForRefresh => write!(f, "Waiting for refresh"),
        }
    }
}

impl std::error::Error for ContourError {}

/// A target contour - the boundary of a detected radar return
#[derive(Debug, Clone)]
pub struct Contour {
    /// Number of points in the contour
    pub length: i32,
    /// Minimum angle in spoke units
    pub min_angle: i32,
    /// Maximum angle in spoke units
    pub max_angle: i32,
    /// Minimum radius in pixels
    pub min_r: i32,
    /// Maximum radius in pixels
    pub max_r: i32,
    /// Center position of the contour
    pub position: Polar,
    /// Points along the contour edge
    pub points: Vec<Polar>,
}

impl Default for Contour {
    fn default() -> Self {
        Self::new()
    }
}

impl Contour {
    /// Create a new empty contour
    pub fn new() -> Self {
        Contour {
            length: 0,
            min_angle: 0,
            max_angle: 0,
            min_r: 0,
            max_r: 0,
            position: Polar::default(),
            points: Vec::new(),
        }
    }

    /// Get the angular width of the contour in spoke units
    pub fn angular_width(&self) -> i32 {
        self.max_angle - self.min_angle
    }

    /// Get the radial extent of the contour in pixels
    pub fn radial_extent(&self) -> i32 {
        self.max_r - self.min_r
    }

    /// Check if contour is within size bounds
    pub fn is_valid(&self) -> bool {
        self.length >= MIN_CONTOUR_LENGTH as i32 && self.length < MAX_CONTOUR_LENGTH as i32 - 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contour_new() {
        let contour = Contour::new();
        assert_eq!(contour.length, 0);
        assert!(contour.points.is_empty());
    }

    #[test]
    fn test_contour_dimensions() {
        let mut contour = Contour::new();
        contour.min_angle = 100;
        contour.max_angle = 150;
        contour.min_r = 50;
        contour.max_r = 80;

        assert_eq!(contour.angular_width(), 50);
        assert_eq!(contour.radial_extent(), 30);
    }

    #[test]
    fn test_contour_validity() {
        let mut contour = Contour::new();
        contour.length = 0;
        assert!(!contour.is_valid());

        contour.length = 10;
        assert!(contour.is_valid());

        contour.length = MAX_CONTOUR_LENGTH as i32;
        assert!(!contour.is_valid());
    }

    #[test]
    fn test_contour_error_display() {
        assert_eq!(
            format!("{}", ContourError::NoContourFound),
            "No contour found"
        );
        assert_eq!(format!("{}", ContourError::Lost), "Target lost");
    }
}
