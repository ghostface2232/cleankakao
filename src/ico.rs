pub fn select_ico_image(bytes: &[u8], desired_size: u32) -> Result<&[u8], String> {
    if bytes.len() < 6 {
        return Err("ICO data is too short".into());
    }

    let reserved = read_u16(bytes, 0)?;
    let image_type = read_u16(bytes, 2)?;
    let count = read_u16(bytes, 4)? as usize;

    if reserved != 0 || image_type != 1 {
        return Err("ICO header is invalid".into());
    }

    let entries_end = 6usize
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| "ICO entry table is too large".to_string())?,
        )
        .ok_or_else(|| "ICO entry table overflows".to_string())?;
    if bytes.len() < entries_end {
        return Err("ICO entry table is truncated".into());
    }

    let mut best: Option<(usize, (u32, u32, u32))> = None;
    for index in 0..count {
        let offset = 6 + index * 16;
        let width = decode_ico_dimension(bytes[offset]);
        let height = decode_ico_dimension(bytes[offset + 1]);
        let size = read_u32(bytes, offset + 8)? as usize;
        let image_offset = read_u32(bytes, offset + 12)? as usize;
        let image_end = image_offset
            .checked_add(size)
            .ok_or_else(|| "ICO image range overflows".to_string())?;

        if image_offset >= bytes.len() || image_end > bytes.len() || size == 0 {
            continue;
        }

        let too_small_penalty = u32::from(width < desired_size || height < desired_size);
        let max_size_delta = width.max(height).abs_diff(desired_size);
        let shape_delta = width.abs_diff(desired_size) + height.abs_diff(desired_size);
        let score = (too_small_penalty, max_size_delta, shape_delta);

        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some((index, score));
        }
    }

    let (index, _) = best.ok_or_else(|| "ICO contains no usable image".to_string())?;
    let offset = 6 + index * 16;
    let size = read_u32(bytes, offset + 8)? as usize;
    let image_offset = read_u32(bytes, offset + 12)? as usize;

    Ok(&bytes[image_offset..image_offset + size])
}

fn decode_ico_dimension(value: u8) -> u32 {
    if value == 0 { 256 } else { value as u32 }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "offset overflows".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "unexpected end of data".to_string())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "offset overflows".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "unexpected end of data".to_string())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_best_image_at_or_above_desired_size() {
        let ico = make_ico(&[
            Entry {
                width: 16,
                height: 16,
                data: b"small",
            },
            Entry {
                width: 32,
                height: 32,
                data: b"desired",
            },
            Entry {
                width: 64,
                height: 64,
                data: b"large",
            },
        ]);

        assert_eq!(select_ico_image(&ico, 32).unwrap(), b"desired");
    }

    #[test]
    fn treats_zero_dimension_as_256() {
        let ico = make_ico(&[
            Entry {
                width: 64,
                height: 64,
                data: b"sixty-four",
            },
            Entry {
                width: 256,
                height: 256,
                data: b"two-fifty-six",
            },
        ]);

        assert_eq!(select_ico_image(&ico, 256).unwrap(), b"two-fifty-six");
    }

    #[test]
    fn rejects_truncated_header_and_table() {
        assert_eq!(
            select_ico_image(&[0, 0, 1], 32).unwrap_err(),
            "ICO data is too short"
        );

        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[32, 32]);
        assert_eq!(
            select_ico_image(&ico, 32).unwrap_err(),
            "ICO entry table is truncated"
        );
    }

    #[test]
    fn skips_unusable_image_ranges() {
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[32, 32, 0, 0, 1, 0, 32, 0, 10, 0, 0, 0, 100, 0, 0, 0]);

        assert_eq!(
            select_ico_image(&ico, 32).unwrap_err(),
            "ICO contains no usable image"
        );
    }

    struct Entry<'a> {
        width: u32,
        height: u32,
        data: &'a [u8],
    }

    fn make_ico(entries: &[Entry<'_>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());

        let mut image_offset = 6 + entries.len() * 16;
        let mut image_data = Vec::new();
        for entry in entries {
            bytes.push(encode_dimension(entry.width));
            bytes.push(encode_dimension(entry.height));
            bytes.push(0);
            bytes.push(0);
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&32u16.to_le_bytes());
            bytes.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(image_offset as u32).to_le_bytes());

            image_data.extend_from_slice(entry.data);
            image_offset += entry.data.len();
        }

        bytes.extend_from_slice(&image_data);
        bytes
    }

    fn encode_dimension(value: u32) -> u8 {
        if value == 256 { 0 } else { value as u8 }
    }
}
