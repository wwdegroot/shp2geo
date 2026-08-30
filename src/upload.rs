//! Multipart upload handling and safe zip extraction.

use std::{
    fs::File,
    io::Write,
    path::{Component, Path, PathBuf},
};

use tempfile::TempDir;

use crate::{error::ApiError, limits::Limits};

/// Extract the uploaded zip into a temporary directory.
///
/// The returned `TempDir` is removed on drop.
pub fn unzip(archive_bytes: &[u8], limits: &Limits) -> Result<TempDir, ApiError> {
    let cursor = std::io::Cursor::new(archive_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| ApiError::InvalidZip(e.to_string()))?;

    let dir = TempDir::new().map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut total_extracted: usize = 0;

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| ApiError::InvalidZip(e.to_string()))?;

        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() {
            return Err(ApiError::UnsafeEntry(format!("symlink '{}'", entry.name())));
        }
        if entry.encrypted() {
            return Err(ApiError::UnsafeEntry(format!(
                "encrypted entry '{}'",
                entry.name()
            )));
        }
        // enclosed_name rejects absolute paths and `..` traversal (zip-slip).
        let rel_path = entry
            .enclosed_name()
            .ok_or_else(|| ApiError::UnsafeEntry(format!("bad path '{}'", entry.name())))?;

        let declared_size = entry.size() as usize;
        total_extracted = total_extracted
            .checked_add(declared_size)
            .filter(|total| *total <= limits.max_extracted_bytes)
            .ok_or(ApiError::ExtractTooLarge {
                max: limits.max_extracted_bytes,
            })?;

        let out_path = sanitize_join(dir.path(), &rel_path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        let mut out_file =
            File::create(&out_path).map_err(|e| ApiError::Internal(e.to_string()))?;

        // Take one byte more than the declared size to detect lying headers.
        let mut limited = std::io::Read::take(entry, declared_size as u64 + 1);
        std::io::copy(&mut limited, &mut out_file).map_err(|e| {
            let _ = std::fs::remove_file(&out_path);
            ApiError::Internal(e.to_string())
        })?;
        let actual = out_file
            .metadata()
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .len() as usize;
        if actual > declared_size {
            let _ = std::fs::remove_file(&out_path);
            return Err(ApiError::ExtractTooLarge {
                max: limits.max_extracted_bytes,
            });
        }
        out_file
            .flush()
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    Ok(dir)
}

/// Join a validated relative path to the root, keeping everything under root
/// even if the path somehow contains a root component after validation.
fn sanitize_join(root: &Path, rel: &Path) -> PathBuf {
    let mut out = root.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => break, // can't happen after enclosed_name(), defensive
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, data) in entries {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                std::io::Write::write_all(&mut w, data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn extracts_ok() {
        let bytes = make_zip(&[("a.shp", b"abc"), ("sub/b.dbf", b"xyz")]);
        let dir = unzip(&bytes, &Limits::default()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.shp")).unwrap(), b"abc");
        assert_eq!(std::fs::read(dir.path().join("sub/b.dbf")).unwrap(), b"xyz");
    }

    #[test]
    fn rejects_non_zip() {
        assert!(matches!(
            unzip(b"not a zip", &Limits::default()),
            Err(ApiError::InvalidZip(_))
        ));
    }

    #[test]
    fn rejects_zip_slip() {
        // Build a zip with a traversal entry name.
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            use zip::ZipWriter;
            use zip::write::SimpleFileOptions;
            let mut w = ZipWriter::new(&mut buf);
            w.start_file("../evil.txt", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut w, b"nope").unwrap();
            w.finish().unwrap();
        }
        let bytes = buf.into_inner();
        assert!(matches!(
            unzip(&bytes, &Limits::default()),
            Err(ApiError::UnsafeEntry(_))
        ));
    }

    #[test]
    fn rejects_extract_too_large() {
        let bytes = make_zip(&[("big.bin", &vec![0u8; 64])]);
        let limits = Limits {
            max_upload_bytes: 1024,
            max_extracted_bytes: 32,
        };
        assert!(matches!(
            unzip(&bytes, &limits),
            Err(ApiError::ExtractTooLarge { .. })
        ));
    }
}
