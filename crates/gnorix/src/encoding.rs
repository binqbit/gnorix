//! Internal versioned framing. This is not a stable storage or network codec.

use std::num::NonZeroUsize;

use crate::{Error, Result, error::Resource};

/// Frame every field (including the domain) with a u64 big-endian byte length.
pub(crate) fn encode(
    domain: &[u8],
    fields: &[&[u8]],
    limit: NonZeroUsize,
    resource: Resource,
) -> Result<Vec<u8>> {
    let length = std::iter::once(domain)
        .chain(fields.iter().copied())
        .try_fold(0usize, |total, field| {
            total.checked_add(8)?.checked_add(field.len())
        })
        .filter(|length| *length <= limit.get())
        .ok_or(Error::LimitExceeded(resource))?;

    let mut bytes = Vec::with_capacity(length);

    for field in std::iter::once(domain).chain(fields.iter().copied()) {
        let size = u64::try_from(field.len()).map_err(|_| Error::LimitExceeded(resource))?;
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(field);
    }

    Ok(bytes)
}
