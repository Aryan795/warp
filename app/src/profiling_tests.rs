use super::*;

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn encode_varint_field(field_number: u64, value: u64, out: &mut Vec<u8>) {
    encode_varint(field_number << 3, out);
    encode_varint(value, out);
}

fn encode_bytes_field(field_number: u64, value: &[u8], out: &mut Vec<u8>) {
    encode_varint((field_number << 3) | 2, out);
    encode_varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

/// Encodes a `Mapping` submessage with an `id` field (to exercise skipping
/// unrelated fields) and, when given, a `build_id` field pointing at the
/// given string-table index.
fn mapping_bytes(build_id_string_table_index: Option<u64>) -> Vec<u8> {
    let mut out = Vec::new();
    const MAPPING_ID_FIELD_NUMBER: u64 = 1;
    const MAPPING_BUILD_ID_FIELD_NUMBER: u64 = 6;
    encode_varint_field(MAPPING_ID_FIELD_NUMBER, 42, &mut out);
    if let Some(index) = build_id_string_table_index {
        encode_varint_field(MAPPING_BUILD_ID_FIELD_NUMBER, index, &mut out);
    }
    out
}

#[test]
fn round_trips_varints() {
    for value in [0u64, 1, 127, 128, 300, 16384, u64::MAX] {
        let mut buf = Vec::new();
        encode_varint(value, &mut buf);
        let mut pos = 0;
        assert_eq!(read_protobuf_varint(&buf, &mut pos).unwrap(), value);
        assert_eq!(pos, buf.len());
    }
}

#[test]
fn truncated_varint_is_an_error() {
    let buf = [0x80]; // Continuation bit set, but no following byte.
    let mut pos = 0;
    assert!(read_protobuf_varint(&buf, &mut pos).is_err());
}

#[test]
fn mapping_with_nonzero_build_id_is_detected() {
    assert!(mapping_has_build_id(&mapping_bytes(Some(7))).unwrap());
}

#[test]
fn mapping_with_no_build_id_field_has_no_build_id() {
    assert!(!mapping_has_build_id(&mapping_bytes(None)).unwrap());
}

#[test]
fn mapping_with_build_id_explicitly_zero_has_no_build_id() {
    // Index 0 always refers to the pprof string table's mandatory empty
    // first entry, so this is indistinguishable from "no build-id".
    assert!(!mapping_has_build_id(&mapping_bytes(Some(0))).unwrap());
}

#[test]
fn counts_mappings_missing_build_id() {
    const PROFILE_MAPPING_FIELD_NUMBER: u64 = 3;
    const PROFILE_STRING_TABLE_FIELD_NUMBER: u64 = 6;

    let mut profile = Vec::new();
    encode_bytes_field(
        PROFILE_MAPPING_FIELD_NUMBER,
        &mapping_bytes(Some(7)),
        &mut profile,
    );
    // A field of a different type interleaved between the two mappings, to
    // confirm the walker skips fields it isn't looking for.
    encode_bytes_field(PROFILE_STRING_TABLE_FIELD_NUMBER, b"", &mut profile);
    encode_bytes_field(
        PROFILE_MAPPING_FIELD_NUMBER,
        &mapping_bytes(None),
        &mut profile,
    );

    let (total, missing_build_id) = count_pprof_mappings_missing_build_id(&profile).unwrap();
    assert_eq!(total, 2);
    assert_eq!(missing_build_id, 1);
}

#[test]
fn profile_with_no_mappings_reports_zero() {
    let (total, missing_build_id) = count_pprof_mappings_missing_build_id(&[]).unwrap();
    assert_eq!(total, 0);
    assert_eq!(missing_build_id, 0);
}

#[test]
fn all_mappings_missing_build_id_are_all_counted() {
    const PROFILE_MAPPING_FIELD_NUMBER: u64 = 3;

    let mut profile = Vec::new();
    for _ in 0..4 {
        encode_bytes_field(
            PROFILE_MAPPING_FIELD_NUMBER,
            &mapping_bytes(None),
            &mut profile,
        );
    }

    let (total, missing_build_id) = count_pprof_mappings_missing_build_id(&profile).unwrap();
    assert_eq!(total, 4);
    assert_eq!(missing_build_id, 4);
}

#[test]
fn truncated_length_delimited_field_is_an_error() {
    let mut profile = Vec::new();
    const PROFILE_MAPPING_FIELD_NUMBER: u64 = 3;
    encode_varint((PROFILE_MAPPING_FIELD_NUMBER << 3) | 2, &mut profile);
    // Claim a length far longer than the (missing) payload that follows.
    encode_varint(100, &mut profile);

    assert!(count_pprof_mappings_missing_build_id(&profile).is_err());
}
