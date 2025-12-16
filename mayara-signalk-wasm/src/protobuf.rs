//! Manual protobuf encoding for RadarMessage
//!
//! Simple protobuf encoder without external dependencies.
//! Based on RadarMessage.proto schema.

use mayara_core::protocol::furuno::ParsedSpoke;

/// Protobuf wire types
const WIRE_TYPE_VARINT: u8 = 0;
const WIRE_TYPE_LENGTH_DELIMITED: u8 = 2;

/// Encode a varint (variable-length integer)
fn encode_varint(mut value: u64, buf: &mut Vec<u8>) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

/// Encode a field tag (field number + wire type)
fn encode_tag(field_number: u32, wire_type: u8, buf: &mut Vec<u8>) {
    encode_varint(((field_number as u64) << 3) | (wire_type as u64), buf);
}

/// Encode a uint32 field
fn encode_uint32(field_number: u32, value: u32, buf: &mut Vec<u8>) {
    encode_tag(field_number, WIRE_TYPE_VARINT, buf);
    encode_varint(value as u64, buf);
}

/// Encode an optional uint32 field (only if Some)
fn encode_optional_uint32(field_number: u32, value: Option<u32>, buf: &mut Vec<u8>) {
    if let Some(v) = value {
        encode_uint32(field_number, v, buf);
    }
}

/// Encode a bytes field
fn encode_bytes(field_number: u32, data: &[u8], buf: &mut Vec<u8>) {
    encode_tag(field_number, WIRE_TYPE_LENGTH_DELIMITED, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

/// Encode a length-delimited message (sub-message)
fn encode_message(field_number: u32, message: &[u8], buf: &mut Vec<u8>) {
    encode_tag(field_number, WIRE_TYPE_LENGTH_DELIMITED, buf);
    encode_varint(message.len() as u64, buf);
    buf.extend_from_slice(message);
}

/// Encode a single Spoke message
///
/// Spoke message fields:
/// - 1: angle (uint32)
/// - 2: bearing (optional uint32)
/// - 3: range (uint32)
/// - 4: time (optional uint64)
/// - 5: data (bytes)
/// - 6: lat (optional int64)
/// - 7: lon (optional int64)
fn encode_spoke(spoke: &ParsedSpoke, range_meters: u32, buf: &mut Vec<u8>) {
    let mut spoke_buf = Vec::with_capacity(spoke.data.len() + 20);

    // Field 1: angle
    encode_uint32(1, spoke.angle as u32, &mut spoke_buf);

    // Field 2: bearing (optional)
    encode_optional_uint32(2, spoke.heading.map(|h| h as u32), &mut spoke_buf);

    // Field 3: range
    encode_uint32(3, range_meters, &mut spoke_buf);

    // Field 4: time - skip for now (optional)

    // Field 5: data
    encode_bytes(5, &spoke.data, &mut spoke_buf);

    // Fields 6,7: lat/lon - skip for now (optional)

    // Encode the spoke as a sub-message in field 2 of RadarMessage
    encode_message(2, &spoke_buf, buf);
}

/// Encode a RadarMessage with multiple spokes
///
/// RadarMessage fields:
/// - 1: radar (uint32) - radar ID
/// - 2: spokes (repeated Spoke)
pub fn encode_radar_message(radar_id: u32, spokes: &[ParsedSpoke], range_meters: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(spokes.len() * 1000);

    // Field 1: radar ID
    encode_uint32(1, radar_id, &mut buf);

    // Field 2: repeated spokes
    for spoke in spokes {
        encode_spoke(spoke, range_meters, &mut buf);
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_varint() {
        let mut buf = Vec::new();
        encode_varint(1, &mut buf);
        assert_eq!(buf, vec![0x01]);

        buf.clear();
        encode_varint(300, &mut buf);
        assert_eq!(buf, vec![0xAC, 0x02]);
    }

    #[test]
    fn test_encode_empty_message() {
        let spokes: Vec<ParsedSpoke> = vec![];
        let data = encode_radar_message(1, &spokes, 1000);
        // Should have at least the radar ID field
        assert!(!data.is_empty());
    }
}
