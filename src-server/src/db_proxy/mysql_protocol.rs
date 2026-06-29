use mysql_async::Value;
//use mysql_async::ColumnType;
use mysql_async::consts::ColumnType;

// PACKET BUILDERS (MySQL Server Protocol)


// 1. Build Length Encoded Integer
pub fn lenenc_int(len: u64) -> Vec<u8> {
    if len < 251 {
        vec![len as u8]
    } else if len < 65536 {
        vec![0xfc, len as u8, (len >> 8) as u8]
    } else if len < 16777216 {
        vec![0xfd, len as u8, (len >> 8) as u8, (len >> 16) as u8]
    } else {
        vec![
            0xfe,
            len as u8,
            (len >> 8) as u8,
            (len >> 16) as u8,
            (len >> 24) as u8,
            (len >> 32) as u8,
            (len >> 40) as u8,
            (len >> 48) as u8,
            (len >> 56) as u8,
        ]
    }
}

// 2. Build Length Encoded String
pub fn lenenc_str(data: &[u8]) -> Vec<u8> {
    let mut res = lenenc_int(data.len() as u64);
    res.extend_from_slice(data);
    res
}

// 3. Build Packet Header (3 bytes length + 1 byte seq_id)
pub fn build_packet(seq_id: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut packet = Vec::with_capacity(4 + len);
    packet.push((len & 0xFF) as u8);
    packet.push(((len >> 8) & 0xFF) as u8);
    packet.push(((len >> 16) & 0xFF) as u8);
    packet.push(seq_id);
    packet.extend_from_slice(payload);
    packet
}

// 4. Build OK Packet
pub fn build_ok_packet(seq_id: u8, affected_rows: u64, last_insert_id: u64) -> Vec<u8> {
    let mut payload = vec![0x00]; // OK header
    payload.extend_from_slice(&lenenc_int(affected_rows));
    payload.extend_from_slice(&lenenc_int(last_insert_id));
    payload.extend_from_slice(&[0x02, 0x00]); // Status flags (SERVER_STATUS_AUTOCOMMIT)
    payload.extend_from_slice(&[0x00, 0x00]); // Warnings
    build_packet(seq_id, &payload)
}

// 5. Build Error Packet
pub fn build_error_packet(seq_id: u8, code: u16, msg: &str) -> Vec<u8> {
    let mut payload = vec![0xff]; // Error header
    payload.push((code & 0xFF) as u8);
    payload.push(((code >> 8) & 0xFF) as u8);
    payload.extend_from_slice(b"#HY000"); // SQL State
    payload.extend_from_slice(msg.as_bytes());
    build_packet(seq_id, &payload)
}

// 6. Build EOF Packet
pub fn build_eof_packet(seq_id: u8) -> Vec<u8> {
    let payload = vec![0xfe, 0x00, 0x00, 0x02, 0x00];
    build_packet(seq_id, &payload)
}

// 7. Build Column Count Packet
pub fn build_column_count_packet(seq_id: u8, count: u64) -> Vec<u8> {
    let payload = lenenc_int(count);
    build_packet(seq_id, &payload)
}

// 8. Build Column Definition Packet
pub fn build_column_def_packet(seq_id: u8, name: &str, col_type: ColumnType) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&lenenc_str(b"def")); // catalog
    payload.extend_from_slice(&lenenc_str(b""));    // schema
    payload.extend_from_slice(&lenenc_str(b""));    // table
    payload.extend_from_slice(&lenenc_str(b""));    // org_table
    payload.extend_from_slice(&lenenc_str(name.as_bytes())); // name
    payload.extend_from_slice(&lenenc_str(name.as_bytes())); // org_name
    payload.push(0x0c); // filler
    payload.extend_from_slice(&[0x21, 0x00]); // charset (utf8_general_ci)
    payload.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]); // column length
    payload.push(map_column_type(col_type)); // column type
    payload.extend_from_slice(&[0x00, 0x00]); // flags
    payload.push(0x00); // decimals
    payload.extend_from_slice(&[0x00, 0x00]); // filler
    build_packet(seq_id, &payload)
}

// 9. Build Row Data Packet
pub fn build_row_packet(seq_id: u8, values: &[Value]) -> Vec<u8> {
    let mut payload = Vec::new();
    for val in values {
        match val {
            Value::NULL => payload.push(0xfb), // NULL byte
            Value::Bytes(b) => payload.extend_from_slice(&lenenc_str(b)),
            Value::Int(i) => payload.extend_from_slice(&lenenc_str(i.to_string().as_bytes())),
            Value::UInt(u) => payload.extend_from_slice(&lenenc_str(u.to_string().as_bytes())),
            Value::Float(f) => payload.extend_from_slice(&lenenc_str(f.to_string().as_bytes())),
            Value::Double(d) => payload.extend_from_slice(&lenenc_str(d.to_string().as_bytes())),
            _ => payload.extend_from_slice(&lenenc_str(b"?")),
        }
    }
    build_packet(seq_id, &payload)
}

// Helper: Map mysql_async ColumnType to MySQL Protocol Type ID
fn map_column_type(t: ColumnType) -> u8 {
    match t {
        ColumnType::MYSQL_TYPE_TINY => 1,
        ColumnType::MYSQL_TYPE_SHORT => 2,
        ColumnType::MYSQL_TYPE_LONG => 3,
        ColumnType::MYSQL_TYPE_FLOAT => 4,
        ColumnType::MYSQL_TYPE_DOUBLE => 5,
        ColumnType::MYSQL_TYPE_NULL => 6,
        ColumnType::MYSQL_TYPE_LONGLONG => 8,
        ColumnType::MYSQL_TYPE_INT24 => 9,
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => 10,
        ColumnType::MYSQL_TYPE_TIME => 11,
        ColumnType::MYSQL_TYPE_DATETIME | ColumnType::MYSQL_TYPE_TIMESTAMP => 12,
        ColumnType::MYSQL_TYPE_YEAR => 13,
        ColumnType::MYSQL_TYPE_VARCHAR | ColumnType::MYSQL_TYPE_VAR_STRING => 15,
        ColumnType::MYSQL_TYPE_BIT => 16,
        ColumnType::MYSQL_TYPE_JSON => 245,
        ColumnType::MYSQL_TYPE_NEWDECIMAL => 246,
        ColumnType::MYSQL_TYPE_ENUM => 247,
        ColumnType::MYSQL_TYPE_SET => 248,
        ColumnType::MYSQL_TYPE_BLOB | ColumnType::MYSQL_TYPE_TINY_BLOB | 
        ColumnType::MYSQL_TYPE_MEDIUM_BLOB | ColumnType::MYSQL_TYPE_LONG_BLOB => 252,
        ColumnType::MYSQL_TYPE_STRING => 253,
        ColumnType::MYSQL_TYPE_GEOMETRY => 255,
        _ => 253, // Default to STRING
    }
}
