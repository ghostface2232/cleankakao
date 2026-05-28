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
