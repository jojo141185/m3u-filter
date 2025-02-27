use std::str::FromStr;

// pub fn parse_size_base_10(size_str: &str) -> Result<u64, String> {
//     let units = [
//         ("KB", 1_000u64),         // Kilobytes (Base 10)
//         ("MB", 1_000_000u64),     // Megabytes
//         ("GB", 1_000_000_000u64), // Gigabytes
//         ("TB", 1_000_000_000_000u64), // Terabytes
//         ("B", 1u64),              // Bytes
//     ];
//
//     let size_str = size_str.trim().to_uppercase();
//
//     for (unit, multiplier) in &units {
//         if size_str.ends_with(unit) {
//             let number_part = size_str[..size_str.len() - unit.len()].trim();
//             let value = u64::from_str(number_part).map_err(|_| format!("Invalid size: {number_part}"))?;
//             return value
//                 .checked_mul(*multiplier)
//                 .ok_or_else(|| format!("Size too large: {size_str}"));
//         }
//     }
//
//     u64::from_str(&size_str).map_err(|_| format!("Invalid size: {size_str}"))
// }

pub fn parse_size_base_2(size_str: &str) -> Result<u64, String> {
    let units = [
        ("KB", 1_024u64),         // Kilobytes
        ("MB", 1_048_576u64),     // Megabytes
        ("GB", 1_073_741_824u64), // Gigabytes
        ("TB", 1_099_511_628_000u64), // Terabytes
        ("B", 1u64),              // Bytes
    ];

    let size_str = size_str.trim().to_uppercase();

    for (unit, multiplier) in &units {
        if size_str.ends_with(unit) {
            let number_part = size_str[..size_str.len() - unit.len()].trim();
            let value = u64::from_str(number_part).map_err(|_| format!("Invalid size: {number_part}"))?;
            return value
                .checked_mul(*multiplier)
                .ok_or_else(|| format!("Size too large: {size_str}"));
        }
    }

    u64::from_str(&size_str).map_err(|_| format!("Invalid size: {size_str}"))
}

pub fn human_readable_byte_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss)]
    let mut size = bytes as f64;
    let mut unit = units[0];

    for next_unit in units.iter().skip(1) {
        if size < 1024.0 {
            break;
        }
        size /= 1024.0;
        unit = next_unit;
    }

    format!("{size:.2} {unit}")
}