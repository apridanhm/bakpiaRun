use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::{Column, Value};

// =====================================================================
// MySQL 4.1 Wire Protocol Constants (Opcodes)
// =====================================================================
pub const COM_QUIT: u8 = 0x01;
pub const COM_INIT_DB: u8 = 0x02;
pub const COM_QUERY: u8 = 0x03;
pub const COM_STATISTICS: u8 = 0x09;
pub const COM_PING: u8 = 0x0E;
pub const COM_STMT_PREPARE: u8 = 0x16;
pub const COM_STMT_EXECUTE: u8 = 0x17;
pub const COM_STMT_SEND_LONG_DATA: u8 = 0x18;
pub const COM_STMT_CLOSE: u8 = 0x19;
pub const COM_STMT_RESET: u8 = 0x1A;

// =====================================================================
// Basic Framing & Length-Encoded Primitives
// =====================================================================

pub fn lenenc_int(val: u64) -> Vec<u8> {
    if val < 251 {
        vec![val as u8]
    } else if val < 65_536 {
        vec![0xfc, val as u8, (val >> 8) as u8]
    } else if val < 16_777_216 {
        vec![0xfd, val as u8, (val >> 8) as u8, (val >> 16) as u8]
    } else {
        vec![
            0xfe,
            val as u8,
            (val >> 8) as u8,
            (val >> 16) as u8,
            (val >> 24) as u8,
            (val >> 32) as u8,
            (val >> 40) as u8,
            (val >> 48) as u8,
            (val >> 56) as u8,
        ]
    }
}

pub fn lenenc_str(data: &[u8]) -> Vec<u8> {
    let mut res = lenenc_int(data.len() as u64);
    res.extend_from_slice(data);
    res
}

/// Reads a length-encoded byte slice, returning `(bytes, total_bytes_consumed)`.
pub fn read_lenenc_bytes(data: &[u8]) -> Result<(&[u8], usize), String> {
    if data.is_empty() {
        return Err("Unexpected EOF reading length-encoded bytes".to_string());
    }

    let first = data[0];
    let (len, hdr_len) = if first < 251 {
        (first as usize, 1usize)
    } else if first == 0xfc {
        if data.len() < 3 {
            return Err("Truncated 2-byte lenenc_int".to_string());
        }
        ((data[1] as usize) | ((data[2] as usize) << 8), 3usize)
    } else if first == 0xfd {
        if data.len() < 4 {
            return Err("Truncated 3-byte lenenc_int".to_string());
        }
        (
            (data[1] as usize) | ((data[2] as usize) << 8) | ((data[3] as usize) << 16),
            4usize,
        )
    } else if first == 0xfe {
        if data.len() < 9 {
            return Err("Truncated 8-byte lenenc_int".to_string());
        }
        let mut l = 0u64;
        for i in 0..8 {
            l |= (data[1 + i] as u64) << (i * 8);
        }
        (l as usize, 9usize)
    } else {
        return Ok((&[], 1));
    };

    if data.len() < hdr_len + len {
        return Err(format!(
            "Truncated lenenc payload: expected {} bytes, got {}",
            len,
            data.len().saturating_sub(hdr_len)
        ));
    }

    Ok((&data[hdr_len..hdr_len + len], hdr_len + len))
}

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

// =====================================================================
// Handshake, OK, EOF, Error Packets
// =====================================================================

pub fn generate_scramble(conn_id: u32) -> [u8; 20] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut state = nanos ^ ((conn_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut scramble = [0u8; 20];
    for byte in &mut scramble {
        // SplitMix64 PRNG step (non-zero printable/safe bytes for MySQL scramble)
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z = z ^ (z >> 31);
        let b = (z & 0x7F) as u8;
        *byte = if b == 0 { 0x21 } else { b };
    }
    scramble
}

pub fn build_handshake_v10(conn_id: u32, scramble: &[u8; 20], version: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(96);
    payload.push(10); // protocol version 10
    payload.extend_from_slice(version.as_bytes());
    payload.push(0); // NUL terminator
    payload.extend_from_slice(&conn_id.to_le_bytes()); // connection id (4 bytes)
    payload.extend_from_slice(&scramble[..8]); // auth-plugin-data-part-1 (8 bytes)
    payload.push(0); // filler

    // Capability flags lower 2 bytes:
    // LONG_PASSWORD | FOUND_ROWS | LONG_FLAG | CONNECT_WITH_DB | NO_SCHEMA | ODBC | PROTOCOL_41 | TRANSACTIONS | SECURE_CONNECTION
    // (No CLIENT_COMPRESS / No CLIENT_SSL so local loopback stays fast plaintext)
    let cap_lower: u16 = 0x0001 | 0x0002 | 0x0004 | 0x0008 | 0x0010 | 0x0040 | 0x0200 | 0x2000 | 0x8000;
    payload.extend_from_slice(&cap_lower.to_le_bytes());

    payload.push(0x2D); // character set: utf8mb4_general_ci (45)
    payload.extend_from_slice(&0x0002u16.to_le_bytes()); // status flags: SERVER_STATUS_AUTOCOMMIT

    // Capability flags upper 2 bytes:
    // MULTI_STATEMENTS | MULTI_RESULTS | PS_MULTI_RESULTS | PLUGIN_AUTH (No CLIENT_DEPRECATE_EOF)
    let cap_upper: u16 = 0x0001 | 0x0002 | 0x0004 | 0x0008;
    payload.extend_from_slice(&cap_upper.to_le_bytes());

    payload.push(21); // auth_plugin_data_len (8 + 12 + 1)
    payload.extend_from_slice(&[0u8; 10]); // reserved 10 zero bytes
    payload.extend_from_slice(&scramble[8..20]); // auth-plugin-data-part-2 (12 bytes)
    payload.push(0); // NUL terminator for auth-plugin-data-part-2
    payload.extend_from_slice(b"mysql_native_password\0");

    build_packet(0, &payload)
}

pub fn build_ok_packet(seq_id: u8, affected_rows: u64, last_insert_id: u64, status_flags: u16) -> Vec<u8> {
    let mut payload = vec![0x00];
    payload.extend_from_slice(&lenenc_int(affected_rows));
    payload.extend_from_slice(&lenenc_int(last_insert_id));
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&[0x00, 0x00]); // warnings
    build_packet(seq_id, &payload)
}

pub fn build_eof_packet(seq_id: u8, status_flags: u16) -> Vec<u8> {
    let mut payload = Vec::with_capacity(5);
    payload.push(0xfe);
    payload.extend_from_slice(&[0x00, 0x00]); // warnings
    payload.extend_from_slice(&status_flags.to_le_bytes());
    build_packet(seq_id, &payload)
}

pub fn build_error_packet(seq_id: u8, code: u16, sql_state: &[u8; 5], msg: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(9 + msg.len());
    payload.push(0xff);
    payload.extend_from_slice(&code.to_le_bytes());
    payload.push(b'#');
    payload.extend_from_slice(sql_state);
    payload.extend_from_slice(msg.as_bytes());
    build_packet(seq_id, &payload)
}

pub fn build_lenenc_int_packet(seq_id: u8, val: u64) -> Vec<u8> {
    build_packet(seq_id, &lenenc_int(val))
}

// =====================================================================
// Column Definition & Parameter Definition Packets
// =====================================================================

pub fn build_column_def_from_mysql(seq_id: u8, col: &Column) -> Vec<u8> {
    let mut payload = Vec::with_capacity(64);
    payload.extend_from_slice(&lenenc_str(b"def"));
    payload.extend_from_slice(&lenenc_str(&col.schema_ref()));
    payload.extend_from_slice(&lenenc_str(&col.table_ref()));
    payload.extend_from_slice(&lenenc_str(&col.org_table_ref()));
    payload.extend_from_slice(&lenenc_str(&col.name_ref()));
    payload.extend_from_slice(&lenenc_str(&col.org_name_ref()));
    payload.push(0x0c); // length of fixed-length fields (12)
    payload.extend_from_slice(&col.character_set().to_le_bytes());
    payload.extend_from_slice(&col.column_length().to_le_bytes());
    payload.push(col.column_type() as u8);
    payload.extend_from_slice(&col.flags().bits().to_le_bytes());
    payload.push(col.decimals());
    payload.extend_from_slice(&[0x00, 0x00]); // filler
    build_packet(seq_id, &payload)
}

pub fn build_stmt_param_def_packet(seq_id: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&lenenc_str(b"def"));
    payload.extend_from_slice(&lenenc_str(b""));
    payload.extend_from_slice(&lenenc_str(b""));
    payload.extend_from_slice(&lenenc_str(b""));
    payload.extend_from_slice(&lenenc_str(b"?"));
    payload.extend_from_slice(&lenenc_str(b""));
    payload.push(0x0c);
    payload.extend_from_slice(&0x003fu16.to_le_bytes()); // binary/general charset
    payload.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    payload.push(ColumnType::MYSQL_TYPE_VAR_STRING as u8);
    payload.extend_from_slice(&[0x00, 0x00]);
    payload.push(0x00);
    payload.extend_from_slice(&[0x00, 0x00]);
    build_packet(seq_id, &payload)
}

// =====================================================================
// Text Protocol Row Packet (COM_QUERY) — Full Date/Time/NULL/Bytes Support
// =====================================================================

pub fn format_value_as_text(val: &Value) -> Option<Vec<u8>> {
    match val {
        Value::NULL => None,
        Value::Bytes(b) => Some(b.clone()),
        Value::Int(i) => Some(i.to_string().into_bytes()),
        Value::UInt(u) => Some(u.to_string().into_bytes()),
        Value::Float(f) => Some(f.to_string().into_bytes()),
        Value::Double(d) => Some(d.to_string().into_bytes()),
        Value::Date(year, month, day, hour, min, sec, micro) => {
            if *hour == 0 && *min == 0 && *sec == 0 && *micro == 0 {
                Some(format!("{:04}-{:02}-{:02}", year, month, day).into_bytes())
            } else if *micro == 0 {
                Some(
                    format!(
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        year, month, day, hour, min, sec
                    )
                    .into_bytes(),
                )
            } else {
                Some(
                    format!(
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
                        year, month, day, hour, min, sec, micro
                    )
                    .into_bytes(),
                )
            }
        }
        Value::Time(is_neg, days, hours, mins, secs, micro) => {
            let sign = if *is_neg { "-" } else { "" };
            let total_hours = (*days) * 24 + (*hours as u32);
            if *micro == 0 {
                Some(format!("{}{:02}:{:02}:{:02}", sign, total_hours, mins, secs).into_bytes())
            } else {
                Some(
                    format!(
                        "{}{:02}:{:02}:{:02}.{:06}",
                        sign, total_hours, mins, secs, micro
                    )
                    .into_bytes(),
                )
            }
        }
    }
}

pub fn build_text_row_packet(seq_id: u8, values: &[Value]) -> Vec<u8> {
    let mut payload = Vec::new();
    for val in values {
        match format_value_as_text(val) {
            None => payload.push(0xfb), // NULL in MySQL Text Resultset
            Some(bytes) => payload.extend_from_slice(&lenenc_str(&bytes)),
        }
    }
    build_packet(seq_id, &payload)
}

// =====================================================================
// Binary Protocol Row Packet (COM_STMT_EXECUTE) — Exact ColumnType Matching
// =====================================================================

pub fn build_binary_row_packet(seq_id: u8, columns: &[Column], values: &[Value]) -> Vec<u8> {
    let col_count = columns.len();
    let null_bitmap_len = (col_count + 7 + 2) / 8;
    let mut null_bitmap = vec![0u8; null_bitmap_len];
    let mut row_bytes = Vec::new();

    for i in 0..col_count {
        let val = values.get(i).unwrap_or(&Value::NULL);
        if matches!(val, Value::NULL) {
            let bit_pos = i + 2;
            null_bitmap[bit_pos / 8] |= 1u8 << (bit_pos % 8);
            continue;
        }

        let col = &columns[i];
        let col_type = col.column_type();
        let is_unsigned = col.flags().contains(ColumnFlags::UNSIGNED_FLAG);

        encode_binary_column_value(&mut row_bytes, col_type, is_unsigned, val);
    }

    let mut payload = Vec::with_capacity(1 + null_bitmap.len() + row_bytes.len());
    payload.push(0x00); // Binary row packet header
    payload.extend_from_slice(&null_bitmap);
    payload.extend_from_slice(&row_bytes);
    build_packet(seq_id, &payload)
}

fn value_as_i64(val: &Value) -> i64 {
    match val {
        Value::Int(i) => *i,
        Value::UInt(u) => *u as i64,
        Value::Bytes(b) => std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0),
        Value::Float(f) => *f as i64,
        Value::Double(d) => *d as i64,
        _ => 0,
    }
}

fn value_as_u64(val: &Value) -> u64 {
    match val {
        Value::UInt(u) => *u,
        Value::Int(i) => *i as u64,
        Value::Bytes(b) => std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
        Value::Float(f) => *f as u64,
        Value::Double(d) => *d as u64,
        _ => 0,
    }
}

fn encode_binary_column_value(
    out: &mut Vec<u8>,
    col_type: ColumnType,
    is_unsigned: bool,
    val: &Value,
) {
    match col_type {
        ColumnType::MYSQL_TYPE_TINY => {
            if is_unsigned {
                out.push(value_as_u64(val) as u8);
            } else {
                out.push((value_as_i64(val) as i8) as u8);
            }
        }
        ColumnType::MYSQL_TYPE_SHORT | ColumnType::MYSQL_TYPE_YEAR => {
            if is_unsigned {
                out.extend_from_slice(&(value_as_u64(val) as u16).to_le_bytes());
            } else {
                out.extend_from_slice(&(value_as_i64(val) as i16).to_le_bytes());
            }
        }
        ColumnType::MYSQL_TYPE_LONG | ColumnType::MYSQL_TYPE_INT24 => {
            if is_unsigned {
                out.extend_from_slice(&(value_as_u64(val) as u32).to_le_bytes());
            } else {
                out.extend_from_slice(&(value_as_i64(val) as i32).to_le_bytes());
            }
        }
        ColumnType::MYSQL_TYPE_LONGLONG => {
            if is_unsigned {
                out.extend_from_slice(&value_as_u64(val).to_le_bytes());
            } else {
                out.extend_from_slice(&value_as_i64(val).to_le_bytes());
            }
        }
        ColumnType::MYSQL_TYPE_FLOAT => {
            let f: f32 = match val {
                Value::Float(v) => *v,
                Value::Double(v) => *v as f32,
                Value::Bytes(b) => std::str::from_utf8(b)
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.0),
                _ => value_as_i64(val) as f32,
            };
            out.extend_from_slice(&f.to_le_bytes());
        }
        ColumnType::MYSQL_TYPE_DOUBLE => {
            let d: f64 = match val {
                Value::Double(v) => *v,
                Value::Float(v) => *v as f64,
                Value::Bytes(b) => std::str::from_utf8(b)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0),
                _ => value_as_i64(val) as f64,
            };
            out.extend_from_slice(&d.to_le_bytes());
        }
        ColumnType::MYSQL_TYPE_DATE
        | ColumnType::MYSQL_TYPE_NEWDATE
        | ColumnType::MYSQL_TYPE_DATETIME
        | ColumnType::MYSQL_TYPE_TIMESTAMP
        | ColumnType::MYSQL_TYPE_DATETIME2
        | ColumnType::MYSQL_TYPE_TIMESTAMP2 => match val {
            Value::Date(year, month, day, hour, min, sec, micro) => {
                if *year == 0 && *month == 0 && *day == 0 && *hour == 0 && *min == 0 && *sec == 0 && *micro == 0 {
                    out.push(0);
                } else if *hour == 0 && *min == 0 && *sec == 0 && *micro == 0 {
                    out.push(4);
                    out.extend_from_slice(&year.to_le_bytes());
                    out.push(*month);
                    out.push(*day);
                } else if *micro == 0 {
                    out.push(7);
                    out.extend_from_slice(&year.to_le_bytes());
                    out.push(*month);
                    out.push(*day);
                    out.push(*hour);
                    out.push(*min);
                    out.push(*sec);
                } else {
                    out.push(11);
                    out.extend_from_slice(&year.to_le_bytes());
                    out.push(*month);
                    out.push(*day);
                    out.push(*hour);
                    out.push(*min);
                    out.push(*sec);
                    out.extend_from_slice(&micro.to_le_bytes());
                }
            }
            _ => {
                let text = format_value_as_text(val).unwrap_or_default();
                out.extend_from_slice(&lenenc_str(&text));
            }
        },
        ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => match val {
            Value::Time(is_neg, days, hours, mins, secs, micro) => {
                if !*is_neg && *days == 0 && *hours == 0 && *mins == 0 && *secs == 0 && *micro == 0 {
                    out.push(0);
                } else if *micro == 0 {
                    out.push(8);
                    out.push(if *is_neg { 1 } else { 0 });
                    out.extend_from_slice(&days.to_le_bytes());
                    out.push(*hours);
                    out.push(*mins);
                    out.push(*secs);
                } else {
                    out.push(12);
                    out.push(if *is_neg { 1 } else { 0 });
                    out.extend_from_slice(&days.to_le_bytes());
                    out.push(*hours);
                    out.push(*mins);
                    out.push(*secs);
                    out.extend_from_slice(&micro.to_le_bytes());
                }
            }
            _ => {
                let text = format_value_as_text(val).unwrap_or_default();
                out.extend_from_slice(&lenenc_str(&text));
            }
        },
        _ => {
            // VARCHAR, VAR_STRING, STRING, DECIMAL, NEWDECIMAL, BLOB, JSON, ENUM, SET, BIT
            let bytes = format_value_as_text(val).unwrap_or_default();
            out.extend_from_slice(&lenenc_str(&bytes));
        }
    }
}

// =====================================================================
// Binary Parameter Parser for COM_STMT_EXECUTE (0x17) -> Vec<mysql_async::Value>
// =====================================================================

/// Parses binary parameters from a `COM_STMT_EXECUTE` packet payload (`payload[0] == 0x17`).
/// Updates `stored_types` when `new_params_bound_flag == 1`, and merges any `long_data` chunks
/// sent earlier via `COM_STMT_SEND_LONG_DATA` (`0x18`).
pub fn decode_stmt_execute_params(
    payload: &[u8],
    num_params: usize,
    stored_types: &mut Vec<(u8, bool)>,
    long_data: &std::collections::HashMap<u16, Vec<u8>>,
) -> Result<Vec<Value>, String> {
    if num_params == 0 {
        return Ok(Vec::new());
    }
    // Header: [0x17 (1)] + [stmt_id (4)] + [flags (1)] + [iteration_count (4)] = 10 bytes
    if payload.len() < 10 {
        return Err("COM_STMT_EXECUTE packet too short (< 10 bytes)".to_string());
    }

    let mut offset = 10usize;
    let bitmap_len = (num_params + 7) / 8;
    if payload.len() < offset + bitmap_len + 1 {
        return Err("COM_STMT_EXECUTE truncated null_bitmap / new_params_bound".to_string());
    }

    let null_bitmap = &payload[offset..offset + bitmap_len];
    offset += bitmap_len;

    let new_params_bound = payload[offset] == 1;
    offset += 1;

    if new_params_bound {
        if payload.len() < offset + num_params * 2 {
            return Err("COM_STMT_EXECUTE truncated parameter type descriptors".to_string());
        }
        stored_types.clear();
        stored_types.reserve(num_params);
        for i in 0..num_params {
            let field_type = payload[offset + i * 2];
            let is_unsigned = (payload[offset + i * 2 + 1] & 0x80) != 0;
            stored_types.push((field_type, is_unsigned));
        }
        offset += num_params * 2;
    } else if stored_types.len() != num_params {
        return Err("COM_STMT_EXECUTE missing bound parameter types from initial execution".to_string());
    }

    let mut values = Vec::with_capacity(num_params);

    for i in 0..num_params {
        // 1. If long_data was streamed via COM_STMT_SEND_LONG_DATA, it supplies this parameter directly
        if let Some(blob) = long_data.get(&(i as u16)) {
            values.push(Value::Bytes(blob.clone()));
            continue;
        }

        // 2. Check NULL bitmap
        if (null_bitmap[i / 8] & (1u8 << (i % 8))) != 0 {
            values.push(Value::NULL);
            continue;
        }

        let (field_type, is_unsigned) = stored_types[i];
        match field_type {
            0x06 => {
                // MYSQL_TYPE_NULL
                values.push(Value::NULL);
            }
            0x01 => {
                // MYSQL_TYPE_TINY (1 byte)
                if offset + 1 > payload.len() {
                    return Err("Truncated TINY parameter".to_string());
                }
                let b = payload[offset];
                offset += 1;
                if is_unsigned {
                    values.push(Value::UInt(b as u64));
                } else {
                    values.push(Value::Int((b as i8) as i64));
                }
            }
            0x02 | 0x0D => {
                // MYSQL_TYPE_SHORT | MYSQL_TYPE_YEAR (2 bytes)
                if offset + 2 > payload.len() {
                    return Err("Truncated SHORT parameter".to_string());
                }
                let raw = [payload[offset], payload[offset + 1]];
                offset += 2;
                if is_unsigned {
                    values.push(Value::UInt(u16::from_le_bytes(raw) as u64));
                } else {
                    values.push(Value::Int(i16::from_le_bytes(raw) as i64));
                }
            }
            0x03 | 0x09 => {
                // MYSQL_TYPE_LONG | MYSQL_TYPE_INT24 (4 bytes)
                if offset + 4 > payload.len() {
                    return Err("Truncated LONG parameter".to_string());
                }
                let raw = [
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ];
                offset += 4;
                if is_unsigned {
                    values.push(Value::UInt(u32::from_le_bytes(raw) as u64));
                } else {
                    values.push(Value::Int(i32::from_le_bytes(raw) as i64));
                }
            }
            0x08 => {
                // MYSQL_TYPE_LONGLONG (8 bytes)
                if offset + 8 > payload.len() {
                    return Err("Truncated LONGLONG parameter".to_string());
                }
                let raw = [
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                    payload[offset + 4],
                    payload[offset + 5],
                    payload[offset + 6],
                    payload[offset + 7],
                ];
                offset += 8;
                if is_unsigned {
                    values.push(Value::UInt(u64::from_le_bytes(raw)));
                } else {
                    values.push(Value::Int(i64::from_le_bytes(raw)));
                }
            }
            0x04 => {
                // MYSQL_TYPE_FLOAT (4 bytes)
                if offset + 4 > payload.len() {
                    return Err("Truncated FLOAT parameter".to_string());
                }
                let raw = [
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ];
                offset += 4;
                values.push(Value::Float(f32::from_le_bytes(raw)));
            }
            0x05 => {
                // MYSQL_TYPE_DOUBLE (8 bytes)
                if offset + 8 > payload.len() {
                    return Err("Truncated DOUBLE parameter".to_string());
                }
                let raw = [
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                    payload[offset + 4],
                    payload[offset + 5],
                    payload[offset + 6],
                    payload[offset + 7],
                ];
                offset += 8;
                values.push(Value::Double(f64::from_le_bytes(raw)));
            }
            0x0A | 0x07 | 0x0C => {
                // DATE / TIMESTAMP / DATETIME binary encoding
                let (b, consumed) = read_lenenc_bytes(&payload[offset..])?;
                offset += consumed;
                let date_val = match b.len() {
                    0 => Value::Date(0, 0, 0, 0, 0, 0, 0),
                    4 => Value::Date(u16::from_le_bytes([b[0], b[1]]), b[2], b[3], 0, 0, 0, 0),
                    7 => Value::Date(
                        u16::from_le_bytes([b[0], b[1]]),
                        b[2],
                        b[3],
                        b[4],
                        b[5],
                        b[6],
                        0,
                    ),
                    11 => Value::Date(
                        u16::from_le_bytes([b[0], b[1]]),
                        b[2],
                        b[3],
                        b[4],
                        b[5],
                        b[6],
                        u32::from_le_bytes([b[7], b[8], b[9], b[10]]),
                    ),
                    _ => Value::Bytes(b.to_vec()),
                };
                values.push(date_val);
            }
            0x0B => {
                // TIME binary encoding
                let (b, consumed) = read_lenenc_bytes(&payload[offset..])?;
                offset += consumed;
                let time_val = match b.len() {
                    0 => Value::Time(false, 0, 0, 0, 0, 0),
                    8 => Value::Time(
                        b[0] != 0,
                        u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
                        b[5],
                        b[6],
                        b[7],
                        0,
                    ),
                    12 => Value::Time(
                        b[0] != 0,
                        u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
                        b[5],
                        b[6],
                        b[7],
                        u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                    ),
                    _ => Value::Bytes(b.to_vec()),
                };
                values.push(time_val);
            }
            _ => {
                // All string / decimal / blob / json / enum / bit types use length-encoded bytes
                let (b, consumed) = read_lenenc_bytes(&payload[offset..])?;
                offset += consumed;
                values.push(Value::Bytes(b.to_vec()));
            }
        }
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lenenc_roundtrip() {
        for &val in &[0u64, 250, 251, 65535, 65536, 1_000_000] {
            let encoded = lenenc_str(&vec![b'A'; val.min(1024) as usize]);
            let (slice, consumed) = read_lenenc_bytes(&encoded).unwrap();
            assert_eq!(consumed, encoded.len());
            assert_eq!(slice.len(), val.min(1024) as usize);
        }
    }

    #[test]
    fn test_format_datetime_and_null() {
        assert_eq!(format_value_as_text(&Value::NULL), None);
        let dt = Value::Date(2026, 11, 23, 14, 5, 9, 0);
        assert_eq!(
            format_value_as_text(&dt),
            Some(b"2026-11-23 14:05:09".to_vec())
        );
    }

    #[test]
    fn test_decode_stmt_execute_params_and_type_reuse() {
        // Build COM_STMT_EXECUTE for 3 params: (INT 42, NULL, VARCHAR "bakpia?")
        let mut pkt = vec![COM_STMT_EXECUTE];
        pkt.extend_from_slice(&1u32.to_le_bytes()); // stmt_id = 1
        pkt.push(0x00); // flags
        pkt.extend_from_slice(&1u32.to_le_bytes()); // iteration_count = 1
        pkt.push(0b0000_0010); // null_bitmap: param #1 is NULL
        pkt.push(1); // new_params_bound = 1
        // Types: LONG (0x03, signed), VARCHAR (0x0f), VARCHAR (0x0f)
        pkt.extend_from_slice(&[0x03, 0x00, 0x0f, 0x00, 0x0f, 0x00]);
        // Values for non-null params (#0 = 42i32, #2 = "bakpia?")
        pkt.extend_from_slice(&42i32.to_le_bytes());
        pkt.extend_from_slice(&lenenc_str(b"bakpia?"));

        let mut stored_types = Vec::new();
        let long_data = std::collections::HashMap::new();
        let vals = decode_stmt_execute_params(&pkt, 3, &mut stored_types, &long_data).unwrap();
        assert_eq!(vals.len(), 3);
        assert_eq!(vals[0], Value::Int(42));
        assert_eq!(vals[1], Value::NULL);
        assert_eq!(vals[2], Value::Bytes(b"bakpia?".to_vec()));

        // Second execution with new_params_bound = 0 must reuse stored_types!
        let mut pkt2 = vec![COM_STMT_EXECUTE];
        pkt2.extend_from_slice(&1u32.to_le_bytes());
        pkt2.push(0x00);
        pkt2.extend_from_slice(&1u32.to_le_bytes());
        pkt2.push(0); // no NULLs
        pkt2.push(0); // new_params_bound = 0!
        pkt2.extend_from_slice(&99i32.to_le_bytes());
        pkt2.extend_from_slice(&lenenc_str(b"second"));
        pkt2.extend_from_slice(&lenenc_str(b"third"));

        let vals2 = decode_stmt_execute_params(&pkt2, 3, &mut stored_types, &long_data).unwrap();
        assert_eq!(vals2[0], Value::Int(99));
        assert_eq!(vals2[1], Value::Bytes(b"second".to_vec()));
        assert_eq!(vals2[2], Value::Bytes(b"third".to_vec()));
    }
}
