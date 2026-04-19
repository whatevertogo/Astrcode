use crate::ApplicationError;

pub(super) fn validate_cursor_format(cursor: &str) -> Result<(), ApplicationError> {
    let Some((storage_seq, subindex)) = cursor.split_once('.') else {
        return Err(ApplicationError::InvalidArgument(format!(
            "invalid cursor '{cursor}'"
        )));
    };
    if storage_seq.parse::<u64>().is_err() || subindex.parse::<u32>().is_err() {
        return Err(ApplicationError::InvalidArgument(format!(
            "invalid cursor '{cursor}'"
        )));
    }
    Ok(())
}

pub(super) fn cursor_is_after_head(
    requested_cursor: &str,
    latest_cursor: Option<&str>,
) -> Result<bool, ApplicationError> {
    let Some(latest_cursor) = latest_cursor else {
        return Ok(false);
    };
    Ok(parse_cursor(requested_cursor)? > parse_cursor(latest_cursor)?)
}

fn parse_cursor(cursor: &str) -> Result<(u64, u32), ApplicationError> {
    let (storage_seq, subindex) = cursor
        .split_once('.')
        .ok_or_else(|| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    let storage_seq = storage_seq
        .parse::<u64>()
        .map_err(|_| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    let subindex = subindex
        .parse::<u32>()
        .map_err(|_| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    Ok((storage_seq, subindex))
}
