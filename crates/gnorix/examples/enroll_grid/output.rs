//! Exclusive package creation and bounded readback.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use super::Result;

pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(path) {
            return Err(format!("{error}; could not remove incomplete package: {cleanup}").into());
        }
        return Err(error.into());
    }
    Ok(())
}

pub(super) fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let count = u64::try_from(limit)?
        .checked_add(1)
        .ok_or("package byte limit overflow")?;
    let mut bytes = Vec::new();
    File::open(path)?.take(count).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("package readback exceeds the byte limit".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn package_creation_never_overwrites_and_readback_is_bounded() {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "gnorix-enroll-{}-{stamp}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let path = directory.join("fixture.grx");
        write_new(&path, b"package").unwrap();
        assert_eq!(read(&path, 7).unwrap(), b"package");
        assert!(read(&path, 6).is_err());
        assert!(write_new(&path, b"replacement").is_err());
        assert_eq!(read(&path, 7).unwrap(), b"package");
        fs::remove_dir_all(directory).unwrap();
    }
}
