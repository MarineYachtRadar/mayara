//! Mayara SignalK WASM Plugin
//!
//! This plugin provides radar discovery and control for SignalK.
//! It uses mayara-core for protocol parsing and SignalK's socket FFI
//! for network I/O.

mod furuno_controller;
mod locator;
mod protobuf;
mod radar_provider;
mod signalk_ffi;
mod spoke_receiver;

use radar_provider::RadarProvider;
use signalk_ffi::{debug, register_radar_provider, set_status};

// =============================================================================
// Plugin Constants
// =============================================================================

const PLUGIN_ID: &str = "mayara-radar";
const PLUGIN_NAME: &str = "Mayara Radar";
const PLUGIN_SCHEMA: &str = r#"{"type":"object","properties":{}}"#;

// =============================================================================
// Plugin State
// =============================================================================

// WASM is single-threaded, so static mut is safe here.
static mut PROVIDER: Option<RadarProvider> = None;

// =============================================================================
// Memory Management (required exports)
// =============================================================================

/// Allocate memory for string passing from host
#[no_mangle]
pub extern "C" fn allocate(size: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Deallocate memory
#[no_mangle]
pub extern "C" fn deallocate(ptr: *mut u8, size: usize) {
    unsafe {
        let _ = Vec::from_raw_parts(ptr, 0, size);
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Write a string to a buffer, returning bytes written or -1 if buffer too small
fn write_string(s: &str, out_ptr: *mut u8, out_max_len: usize) -> i32 {
    let bytes = s.as_bytes();
    if bytes.len() > out_max_len {
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
    }
    bytes.len() as i32
}

// =============================================================================
// Plugin Exports (required by SignalK WASM runtime)
// =============================================================================

/// Return the plugin ID (buffer-based)
#[no_mangle]
pub extern "C" fn plugin_id(out_ptr: *mut u8, out_max_len: usize) -> i32 {
    write_string(PLUGIN_ID, out_ptr, out_max_len)
}

/// Return the plugin name (buffer-based)
#[no_mangle]
pub extern "C" fn plugin_name(out_ptr: *mut u8, out_max_len: usize) -> i32 {
    write_string(PLUGIN_NAME, out_ptr, out_max_len)
}

/// Return the plugin JSON schema (buffer-based)
#[no_mangle]
pub extern "C" fn plugin_schema(out_ptr: *mut u8, out_max_len: usize) -> i32 {
    write_string(PLUGIN_SCHEMA, out_ptr, out_max_len)
}

/// Start the plugin with configuration
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plugin_start(_config_ptr: *const u8, _config_len: usize) -> i32 {
    debug("Mayara Radar plugin starting...");

    // Try to register as a radar provider (optional - works without it via deltas)
    if register_radar_provider(PLUGIN_NAME) {
        debug("Registered as radar provider");
    } else {
        debug("Could not register as radar provider (capability not granted) - using delta emission only");
    }

    unsafe {
        PROVIDER = Some(RadarProvider::new());
    }

    set_status("Running - scanning for radars");
    debug("Mayara Radar plugin started");
    0 // Success
}

/// Stop the plugin
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plugin_stop() -> i32 {
    debug("Mayara Radar plugin stopping...");

    unsafe {
        if let Some(mut provider) = PROVIDER.take() {
            provider.shutdown();
        }
    }

    set_status("Stopped");
    debug("Mayara Radar plugin stopped");
    0
}

/// Poll function (optional - called every 1 second if exported)
///
/// Called periodically by the SignalK runtime to process network events.
/// Returns 0 on success, negative on error.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn poll() -> i32 {
    static mut POLL_CALL_COUNT: u64 = 0;

    unsafe {
        POLL_CALL_COUNT += 1;

        // Log every 100 calls to confirm poll is being called
        if POLL_CALL_COUNT % 100 == 1 {
            debug(&format!("lib.rs poll() called #{}", POLL_CALL_COUNT));
        }

        if let Some(ref mut provider) = PROVIDER {
            provider.poll()
        } else {
            if POLL_CALL_COUNT <= 5 {
                debug("poll() called but PROVIDER is None!");
            }
            -1
        }
    }
}

// =============================================================================
// Radar Provider Exports (required for SignalK Radar API)
// =============================================================================

/// Return JSON array of radar IDs this provider manages
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_get_radars(out_ptr: *mut u8, out_max_len: usize) -> i32 {
    debug(&format!("radar_get_radars called, buffer size: {}", out_max_len));

    unsafe {
        if let Some(ref provider) = PROVIDER {
            let ids: Vec<&str> = provider.get_radar_ids();
            debug(&format!("radar_get_radars: found {} radars", ids.len()));
            match serde_json::to_string(&ids) {
                Ok(json) => {
                    debug(&format!("radar_get_radars: json len={}, content={}", json.len(), &json));
                    if json.len() > out_max_len {
                        debug(&format!("radar_get_radars: buffer too small! need {} have {}", json.len(), out_max_len));
                        // Return 0 with empty response instead of -1
                        return 0;
                    }
                    write_string(&json, out_ptr, out_max_len)
                }
                Err(e) => {
                    debug(&format!("radar_get_radars: serialize error: {}", e));
                    write_string("[]", out_ptr, out_max_len)
                }
            }
        } else {
            debug("radar_get_radars: provider not initialized");
            write_string("[]", out_ptr, out_max_len)
        }
    }
}

/// Return RadarInfo JSON for a specific radar
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_get_info(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    // Parse request JSON to get radar ID
    let request_str = unsafe {
        let slice = std::slice::from_raw_parts(request_ptr, request_len);
        match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => return write_string(r#"{"error":"invalid utf8"}"#, out_ptr, out_max_len),
        }
    };

    // Parse {"radarId": "..."} from request
    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
    }

    let req: Request = match serde_json::from_str(request_str) {
        Ok(r) => r,
        Err(_) => return write_string(r#"{"error":"invalid request"}"#, out_ptr, out_max_len),
    };

    unsafe {
        if let Some(ref provider) = PROVIDER {
            if let Some(info) = provider.get_radar_info(&req.radar_id) {
                match serde_json::to_string(&info) {
                    Ok(json) => write_string(&json, out_ptr, out_max_len),
                    Err(_) => write_string(r#"{"error":"serialize failed"}"#, out_ptr, out_max_len),
                }
            } else {
                write_string(r#"{"error":"radar not found"}"#, out_ptr, out_max_len)
            }
        } else {
            write_string(r#"{"error":"provider not initialized"}"#, out_ptr, out_max_len)
        }
    }
}

// =============================================================================
// Radar Control Exports (for SignalK REST API)
// =============================================================================

/// Helper to parse request JSON
fn parse_request(request_ptr: *const u8, request_len: usize) -> Result<String, &'static str> {
    unsafe {
        let slice = std::slice::from_raw_parts(request_ptr, request_len);
        std::str::from_utf8(slice)
            .map(|s| s.to_string())
            .map_err(|_| "invalid utf8")
    }
}

/// Set radar power state
/// Request: {"radarId": "...", "state": "off|standby|transmit|warming"}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_power(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        state: String,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!("radar_set_power: {} -> {}", req.radar_id, req.state));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_power(&req.radar_id, &req.state);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

/// Set radar range in meters
/// Request: {"radarId": "...", "range": 1000}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_range(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        range: u32,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!("radar_set_range: {} -> {}m", req.radar_id, req.range));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_range(&req.radar_id, req.range);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

/// Set radar gain
/// Request: {"radarId": "...", "gain": {"auto": true, "value": 50}}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_gain(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct GainValue {
        auto: bool,
        value: Option<u8>,
    }

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        gain: GainValue,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!(
        "radar_set_gain: {} -> auto={}, value={:?}",
        req.radar_id, req.gain.auto, req.gain.value
    ));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_gain(&req.radar_id, req.gain.auto, req.gain.value);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

/// Set radar sea clutter
/// Request: {"radarId": "...", "sea": {"auto": true, "value": 50}}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_sea(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct SeaValue {
        auto: bool,
        value: Option<u8>,
    }

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        sea: SeaValue,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!(
        "radar_set_sea: {} -> auto={}, value={:?}",
        req.radar_id, req.sea.auto, req.sea.value
    ));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_sea(&req.radar_id, req.sea.auto, req.sea.value);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

/// Set radar rain clutter
/// Request: {"radarId": "...", "rain": {"auto": true, "value": 50}}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_rain(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct RainValue {
        auto: bool,
        value: Option<u8>,
    }

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        rain: RainValue,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!(
        "radar_set_rain: {} -> auto={}, value={:?}",
        req.radar_id, req.rain.auto, req.rain.value
    ));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_rain(&req.radar_id, req.rain.auto, req.rain.value);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

/// Set multiple radar controls at once
/// Request: {"radarId": "...", "controls": {"power": "transmit", "range": 1000, ...}}
/// Response: "true" or "false"
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_controls(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        controls: serde_json::Value,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string("false", out_ptr, out_max_len),
    };

    debug(&format!("radar_set_controls: {} -> {:?}", req.radar_id, req.controls));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            let success = provider.set_controls(&req.radar_id, &req.controls);
            write_string(if success { "true" } else { "false" }, out_ptr, out_max_len)
        } else {
            write_string("false", out_ptr, out_max_len)
        }
    }
}

// =============================================================================
// Radar Provider API v5 Exports
// =============================================================================

/// Return CapabilityManifest JSON for a radar (v5 API)
/// Request: {"radarId": "..."}
/// Response: CapabilityManifest JSON or {"error": "..."}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_get_capabilities(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string(r#"{"error":"invalid utf8"}"#, out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string(r#"{"error":"invalid request"}"#, out_ptr, out_max_len),
    };

    debug(&format!("radar_get_capabilities: {}", req.radar_id));

    unsafe {
        if let Some(ref provider) = PROVIDER {
            if let Some(caps) = provider.get_capabilities(&req.radar_id) {
                match serde_json::to_string(&caps) {
                    Ok(json) => write_string(&json, out_ptr, out_max_len),
                    Err(_) => write_string(r#"{"error":"serialize failed"}"#, out_ptr, out_max_len),
                }
            } else {
                write_string(r#"{"error":"radar not found"}"#, out_ptr, out_max_len)
            }
        } else {
            write_string(r#"{"error":"provider not initialized"}"#, out_ptr, out_max_len)
        }
    }
}

/// Return RadarState JSON (v5 format with controls map)
/// Request: {"radarId": "..."}
/// Response: RadarStateV5 JSON or {"error": "..."}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_get_state(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string(r#"{"error":"invalid utf8"}"#, out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string(r#"{"error":"invalid request"}"#, out_ptr, out_max_len),
    };

    debug(&format!("radar_get_state: {}", req.radar_id));

    unsafe {
        if let Some(ref provider) = PROVIDER {
            if let Some(state) = provider.get_state_v5(&req.radar_id) {
                match serde_json::to_string(&state) {
                    Ok(json) => write_string(&json, out_ptr, out_max_len),
                    Err(_) => write_string(r#"{"error":"serialize failed"}"#, out_ptr, out_max_len),
                }
            } else {
                write_string(r#"{"error":"radar not found"}"#, out_ptr, out_max_len)
            }
        } else {
            write_string(r#"{"error":"provider not initialized"}"#, out_ptr, out_max_len)
        }
    }
}

/// Get a single control value (v5 API)
/// Request: {"radarId": "...", "controlId": "gain"}
/// Response: ControlValue JSON or {"error": "..."}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_get_control(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string(r#"{"error":"invalid utf8"}"#, out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        #[serde(rename = "controlId")]
        control_id: String,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string(r#"{"error":"invalid request"}"#, out_ptr, out_max_len),
    };

    debug(&format!("radar_get_control: {} {}", req.radar_id, req.control_id));

    unsafe {
        if let Some(ref provider) = PROVIDER {
            if let Some(value) = provider.get_control(&req.radar_id, &req.control_id) {
                match serde_json::to_string(&value) {
                    Ok(json) => write_string(&json, out_ptr, out_max_len),
                    Err(_) => write_string(r#"{"error":"serialize failed"}"#, out_ptr, out_max_len),
                }
            } else {
                write_string(r#"{"error":"control not found"}"#, out_ptr, out_max_len)
            }
        } else {
            write_string(r#"{"error":"provider not initialized"}"#, out_ptr, out_max_len)
        }
    }
}

/// Set a single control value (v5 generic interface)
/// Request: {"radarId": "...", "controlId": "gain", "value": {...}}
/// Response: {"success": true} or {"success": false, "error": "..."}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn radar_set_control(
    request_ptr: *const u8,
    request_len: usize,
    out_ptr: *mut u8,
    out_max_len: usize,
) -> i32 {
    let request_str = match parse_request(request_ptr, request_len) {
        Ok(s) => s,
        Err(_) => return write_string(r#"{"success":false,"error":"invalid utf8"}"#, out_ptr, out_max_len),
    };

    #[derive(serde::Deserialize)]
    struct Request {
        #[serde(rename = "radarId")]
        radar_id: String,
        #[serde(rename = "controlId")]
        control_id: String,
        value: serde_json::Value,
    }

    let req: Request = match serde_json::from_str(&request_str) {
        Ok(r) => r,
        Err(_) => return write_string(r#"{"success":false,"error":"invalid request"}"#, out_ptr, out_max_len),
    };

    debug(&format!("radar_set_control: {} {} {:?}", req.radar_id, req.control_id, req.value));

    unsafe {
        if let Some(ref mut provider) = PROVIDER {
            match provider.set_control_v5(&req.radar_id, &req.control_id, &req.value) {
                Ok(()) => write_string(r#"{"success":true}"#, out_ptr, out_max_len),
                Err(e) => {
                    let error = format!(r#"{{"success":false,"error":"{}"}}"#, e);
                    write_string(&error, out_ptr, out_max_len)
                }
            }
        } else {
            write_string(r#"{"success":false,"error":"provider not initialized"}"#, out_ptr, out_max_len)
        }
    }
}
