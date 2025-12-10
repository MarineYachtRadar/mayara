//! Spoke Receiver
//!
//! Receives spoke data from discovered radars and emits to SignalK stream.

use mayara_core::protocol::furuno::{self, ParsedSpoke};
use crate::signalk_ffi::{debug, UdpSocket, emit_radar_spokes};
use crate::protobuf::encode_radar_message;

/// Furuno sends 8192 spokes per revolution
/// Set to 1 for full resolution, 4 for reduced (2048 spokes)
const FURUNO_SPOKE_REDUCTION: usize = 4;
/// Output spokes per revolution (8192 / FURUNO_SPOKE_REDUCTION)
pub const FURUNO_OUTPUT_SPOKES: u16 = 2048;

/// State for a single radar's spoke reception
pub struct RadarSpokeState {
    /// Radar ID (e.g., "Furuno-RD003212")
    pub radar_id: String,
    /// Numeric ID for protobuf (simple hash)
    pub numeric_id: u32,
    /// Source IP address to filter by
    pub source_ip: String,
    /// Previous spoke buffer for delta decoding
    pub prev_spoke: Vec<u8>,
    /// Current range in meters
    pub current_range: u32,
    /// Spoke accumulator for combining 4 spokes into 1
    spoke_accumulator: Vec<ParsedSpoke>,
}

/// Spoke receiver for all radars
pub struct SpokeReceiver {
    /// Furuno spoke socket (multicast 239.255.0.2:10024)
    furuno_socket: Option<UdpSocket>,
    /// Active Furuno radars being tracked
    furuno_radars: Vec<RadarSpokeState>,
    /// Receive buffer
    buf: Vec<u8>,
}

impl SpokeReceiver {
    pub fn new() -> Self {
        Self {
            furuno_socket: None,
            furuno_radars: Vec::new(),
            buf: vec![0u8; 9000], // Large buffer for spoke frames
        }
    }

    /// Start listening for Furuno spokes
    pub fn start_furuno(&mut self) {
        if self.furuno_socket.is_some() {
            return; // Already listening
        }

        match UdpSocket::new() {
            Ok(socket) => {
                if socket.bind_port(furuno::DATA_PORT).is_ok() {
                    if socket.join_multicast(furuno::DATA_MULTICAST_ADDR).is_ok() {
                        debug(&format!(
                            "Listening for Furuno spokes on {}:{}",
                            furuno::DATA_MULTICAST_ADDR,
                            furuno::DATA_PORT
                        ));
                        self.furuno_socket = Some(socket);
                    } else {
                        debug("Failed to join Furuno spoke multicast group");
                    }
                } else {
                    debug("Failed to bind Furuno spoke socket");
                }
            }
            Err(e) => {
                debug(&format!("Failed to create Furuno spoke socket: {}", e));
            }
        }
    }

    /// Register a discovered Furuno radar for spoke tracking
    pub fn add_furuno_radar(&mut self, radar_id: &str, source_ip: &str) {
        // Check if already tracking this radar
        if self.furuno_radars.iter().any(|r| r.radar_id == radar_id) {
            return;
        }

        // Simple hash for numeric ID (just use first 4 chars as bytes)
        let numeric_id = radar_id.bytes().take(4).fold(0u32, |acc, b| (acc << 8) | b as u32);

        debug(&format!("Tracking Furuno radar {} from {} (id={})", radar_id, source_ip, numeric_id));

        self.furuno_radars.push(RadarSpokeState {
            radar_id: radar_id.to_string(),
            numeric_id,
            source_ip: source_ip.to_string(),
            prev_spoke: Vec::new(),
            current_range: 1500, // Default 1.5km
            spoke_accumulator: Vec::with_capacity(FURUNO_SPOKE_REDUCTION),
        });

        // Start socket if not already listening
        self.start_furuno();
    }

    /// Poll for incoming spoke data and emit to SignalK
    ///
    /// Returns number of spokes emitted.
    pub fn poll(&mut self) -> u32 {
        let mut total_emitted = 0;
        static mut POLL_COUNT: u64 = 0;
        static mut LAST_LOG: u64 = 0;

        unsafe {
            POLL_COUNT += 1;
        }

        // Collect frames first to avoid borrow conflicts
        let mut frames: Vec<(Vec<u8>, usize)> = Vec::new();
        let mut unknown_packets = 0u32;

        // Poll Furuno spokes
        if let Some(socket) = &self.furuno_socket {
            while let Some((len, addr, _port)) = socket.recv_from(&mut self.buf) {
                // Find which radar this packet is from
                let radar_idx = self.furuno_radars.iter().position(|r| r.source_ip == addr);

                if let Some(idx) = radar_idx {
                    frames.push((self.buf[..len].to_vec(), idx));
                } else {
                    unknown_packets += 1;
                    // Log first unknown packet or periodically
                    unsafe {
                        if POLL_COUNT - LAST_LOG > 100 {
                            debug(&format!("Spoke packet from unknown IP {} (len={}), tracking: {:?}",
                                addr, len,
                                self.furuno_radars.iter().map(|r| r.source_ip.as_str()).collect::<Vec<_>>()
                            ));
                            LAST_LOG = POLL_COUNT;
                        }
                    }
                }
            }
        }

        // Log periodically
        unsafe {
            if POLL_COUNT % 500 == 0 {
                debug(&format!(
                    "SpokeReceiver poll #{}: socket={}, tracking {} radars, frames={}, unknown={}",
                    POLL_COUNT,
                    self.furuno_socket.is_some(),
                    self.furuno_radars.len(),
                    frames.len(),
                    unknown_packets
                ));
            }
        }

        // Process collected frames
        for (data, idx) in frames {
            let emitted = self.process_furuno_frame(&data, idx);
            total_emitted += emitted;
        }

        total_emitted
    }

    /// Combine multiple spokes into one using max() for each pixel
    /// This preserves radar targets while reducing data rate
    fn combine_spokes(spokes: &[ParsedSpoke]) -> ParsedSpoke {
        if spokes.is_empty() {
            return ParsedSpoke {
                angle: 0,
                heading: None,
                data: Vec::new(),
            };
        }

        // Use first spoke's angle (divided by reduction factor) and heading
        let output_angle = spokes[0].angle / FURUNO_SPOKE_REDUCTION as u16;
        let heading = spokes[0].heading;

        // Find max data length
        let max_len = spokes.iter().map(|s| s.data.len()).max().unwrap_or(0);

        // Combine pixel data using max() - preserves targets
        let mut combined_data = vec![0u8; max_len];
        for spoke in spokes {
            for (i, &pixel) in spoke.data.iter().enumerate() {
                if pixel > combined_data[i] {
                    combined_data[i] = pixel;
                }
            }
        }

        ParsedSpoke {
            angle: output_angle,
            heading,
            data: combined_data,
        }
    }

    /// Process a Furuno spoke frame
    fn process_furuno_frame(&mut self, data: &[u8], radar_idx: usize) -> u32 {
        static mut FRAME_COUNT: u64 = 0;
        static mut EMIT_SUCCESS: u64 = 0;
        static mut EMIT_FAIL: u64 = 0;

        unsafe { FRAME_COUNT += 1; }

        if !furuno::is_spoke_frame(data) {
            unsafe {
                if FRAME_COUNT % 100 == 0 {
                    debug(&format!("Frame #{} not a spoke frame (len={})", FRAME_COUNT, data.len()));
                }
            }
            return 0;
        }

        // Parse the header to get range
        if let Ok(header) = furuno::parse_spoke_header(data) {
            let range = furuno::get_range_meters(header.range_index);
            if range > 0 {
                self.furuno_radars[radar_idx].current_range = range;
            }
        }

        // Get mutable reference to radar state
        let radar = &mut self.furuno_radars[radar_idx];
        let radar_id = radar.radar_id.clone();
        let numeric_id = radar.numeric_id;
        let range = radar.current_range;

        // Parse spokes
        match furuno::parse_spoke_frame(data, &mut radar.prev_spoke) {
            Ok(spokes) => {
                if spokes.is_empty() {
                    return 0;
                }

                // Accumulate spokes for reduction (8192 -> 2048)
                // Add all parsed spokes to the accumulator
                for spoke in spokes {
                    radar.spoke_accumulator.push(spoke);
                }

                // Combine when we have enough spokes
                let mut combined_spokes: Vec<ParsedSpoke> = Vec::new();
                while radar.spoke_accumulator.len() >= FURUNO_SPOKE_REDUCTION {
                    // Take first FURUNO_SPOKE_REDUCTION spokes
                    let to_combine: Vec<ParsedSpoke> = radar.spoke_accumulator
                        .drain(..FURUNO_SPOKE_REDUCTION)
                        .collect();
                    combined_spokes.push(Self::combine_spokes(&to_combine));
                }

                if combined_spokes.is_empty() {
                    return 0;
                }

                // Encode combined spokes to protobuf
                let protobuf_data = encode_radar_message(numeric_id, &combined_spokes, range);

                // Log periodically
                unsafe {
                    if FRAME_COUNT % 500 == 0 {
                        debug(&format!(
                            "Frame #{}: {} combined spokes (from {}), range={}m, protobuf={} bytes, emit success/fail={}/{}",
                            FRAME_COUNT, combined_spokes.len(), combined_spokes.len() * FURUNO_SPOKE_REDUCTION,
                            range, protobuf_data.len(),
                            EMIT_SUCCESS, EMIT_FAIL
                        ));
                    }
                }

                // Emit to SignalK
                if emit_radar_spokes(&radar_id, &protobuf_data) {
                    unsafe { EMIT_SUCCESS += 1; }
                    combined_spokes.len() as u32
                } else {
                    unsafe { EMIT_FAIL += 1; }
                    // Log emit failures
                    unsafe {
                        if EMIT_FAIL <= 5 || EMIT_FAIL % 100 == 0 {
                            debug(&format!("emit_radar_spokes failed for {} (fail #{})", radar_id, EMIT_FAIL));
                        }
                    }
                    0
                }
            }
            Err(e) => {
                debug(&format!("Furuno spoke parse error: {}", e));
                0
            }
        }
    }

    /// Shutdown all sockets
    pub fn shutdown(&mut self) {
        self.furuno_socket = None;
        self.furuno_radars.clear();
    }
}

impl Default for SpokeReceiver {
    fn default() -> Self {
        Self::new()
    }
}
