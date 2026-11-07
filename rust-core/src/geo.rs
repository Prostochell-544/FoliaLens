use h3o::{CellIndex, LatLng, Resolution};

/// Converts latitude and longitude into an H3 index at resolution 4 (~11,000 km² cells).
pub fn lat_lon_to_h3_res4(lat: f64, lon: f64) -> Option<i64> {
    let coord = LatLng::new(lat, lon).ok()?;
    let cell: CellIndex = coord.to_cell(Resolution::Four);
    Some(u64::from(cell) as i64)
}

/// Converts a 1-based month (1 = Jan, 12 = Dec) to a 12-bit presence mask.
pub fn month_to_bitmask(month: u8) -> u16 {
    if (1..=12).contains(&month) {
        1u16 << (month - 1)
    } else {
        0
    }
}

/// Returns true if the species is active/observable during `current_month`.
pub fn is_present_in_season(months_bitmask: u16, current_month: u8) -> bool {
    let mask = month_to_bitmask(current_month);
    (months_bitmask & mask) != 0
}

/// Formats an int64 H3 index as a standard hexadecimal string.
pub fn h3_to_hex(h3_index: i64) -> String {
    format!("{:x}", h3_index as u64)
}
