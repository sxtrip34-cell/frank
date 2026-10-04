// Dropped files are copied into %LOCALAPPDATA%\Frank\inbox so the original is
// never touched and the copy survives the drag source going away.
// The inbox is swept of anything older than a week, as on macOS.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::settings;

const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

pub fn inbox_dir() -> PathBuf {
    settings::local_dir().join("inbox")
}

pub fn ingest(source: &str) -> Result<DroppedFile, String> {
    let src = Path::new(source);
    let meta = std::fs::metadata(src).map_err(|e| format!("cannot read {source}: {e}"))?;
    if meta.is_dir() {
        return Err("Folders can't be dropped yet.".into());
    }

    let dir = inbox_dir();
    crate::platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());

    let dest = free_path(&dir, &name);
    std::fs::copy(src, &dest).map_err(|e| format!("cannot copy: {e}"))?;
    // CopyFileEx carries the source's timestamps across, so a file last edited
    // three years ago would arrive already older than the sweep window and be
    // deleted on the spot. The inbox ages from when *we* copied it.
    if let Ok(file) = std::fs::File::options().write(true).open(&dest) {
        let _ = file.set_modified(SystemTime::now());
    }
    sweep(&dir);

    Ok(DroppedFile {
        name,
        path: dest.to_string_lossy().to_string(),
        size: meta.len(),
    })
}

/// Largest pasted image accepted. The island shrinks screenshots well below
/// this before sending them; the cap only stops something absurd.
const MAX_IMAGE: usize = 20 * 1024 * 1024;

/// Saves an image pasted into the chat (a screenshot, usually) into the inbox,
/// where the chat can read it like a dropped file. PNG only: the island always
/// re-encodes what the clipboard hands it to PNG first.
pub fn ingest_image(bytes: &[u8]) -> Result<DroppedFile, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Only images can be pasted.".into());
    }
    if bytes.len() > MAX_IMAGE {
        return Err("That image is too large.".into());
    }

    let dir = inbox_dir();
    crate::platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let t = crate::platform::local_time();
    let name = format!(
        "screenshot-{:04}{:02}{:02}-{:02}{:02}{:02}.png",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let dest = free_path(&dir, &name);
    std::fs::write(&dest, bytes).map_err(|e| format!("cannot save the image: {e}"))?;
    sweep(&dir);

    Ok(DroppedFile {
        name: dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(name),
        path: dest.to_string_lossy().to_string(),
        size: bytes.len() as u64,
    })
}

/// Largest file accepted by drag and drop. It arrives through the page as
/// bytes, so the cap keeps a stray video from being read into memory.
pub const MAX_DROPPED: u64 = 200 * 1024 * 1024;

/// Saves a file dropped on the island. HTML5 drag and drop hands the page the
/// file's name and bytes but not its path, so the bytes are written into the
/// inbox under that name (stripped of anything that could point elsewhere).
pub fn ingest_bytes(name: &str, bytes: &[u8]) -> Result<DroppedFile, String> {
    if bytes.len() as u64 > MAX_DROPPED {
        return Err("That file is too large to drop (200 MB at most).".into());
    }
    // Only the last component, and nothing Windows would refuse or treat as a
    // device: a dropped name is the sender's, not ours.
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .map(|c| if c.is_control() || r#"<>:"|?*"#.contains(c) { '_' } else { c })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let name = if clean.is_empty() { "file".to_string() } else { clean };

    let dir = inbox_dir();
    crate::platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = free_path(&dir, &name);
    std::fs::write(&dest, bytes).map_err(|e| format!("cannot save the file: {e}"))?;
    sweep(&dir);

    Ok(DroppedFile {
        name: dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(name),
        path: dest.to_string_lossy().to_string(),
        size: bytes.len() as u64,
    })
}

/// `dir/name`, or `dir/stem (2).ext` and so on when that is taken: a second
/// file of the same name never overwrites the first.
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    if !dest.exists() {
        return dest;
    }
    let as_path = Path::new(name);
    let stem = as_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = as_path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    (2..1000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(dest)
}

/// Drops anything copied here more than a week ago. `ingest` stamps every copy
/// with the time it landed, so this really is the age of the copy and not the
/// age of whatever the user happened to drag in.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(copied) = meta.modified() else { continue };
        if now.duration_since(copied).map(|age| age > KEEP_FOR).unwrap_or(false) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_copies_and_never_overwrites() {
        let tmp = std::env::temp_dir().join(format!("frank-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let source = tmp.join("note.txt");
        std::fs::write(&source, b"hello").unwrap();

        let first = ingest(source.to_str().unwrap()).unwrap();
        assert_eq!(first.name, "note.txt");
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");

        // A second drop of the same name must not clobber the first copy.
        std::fs::write(&source, b"second").unwrap();
        let second = ingest(source.to_str().unwrap()).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");
        assert_eq!(std::fs::read(&second.path).unwrap(), b"second");

        // Folders are refused rather than silently ignored.
        assert!(ingest(tmp.to_str().unwrap()).is_err());

        // An ancient source must not arrive already older than the sweep window.
        let old_source = tmp.join("ancient.txt");
        std::fs::write(&old_source, b"old").unwrap();
        let long_ago = SystemTime::now() - KEEP_FOR - Duration::from_secs(60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old_source)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let aged = ingest(old_source.to_str().unwrap()).unwrap();
        assert!(
            Path::new(&aged.path).exists(),
            "a file copied just now was swept as if it were a week old"
        );
        let _ = std::fs::remove_file(&aged.path);

        let _ = std::fs::remove_file(&first.path);
        let _ = std::fs::remove_file(&second.path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pasted_images_are_saved_as_png_and_never_overwrite() {
        // Anything that is not a PNG is refused before touching the disk.
        assert!(ingest_image(b"GIF89a....").is_err());
        assert!(ingest_image(b"").is_err());

        let png = b"\x89PNG\r\n\x1a\nfake image body";
        let first = ingest_image(png).unwrap();
        let second = ingest_image(png).unwrap();
        assert!(first.name.starts_with("screenshot-") && first.name.ends_with(".png"));
        // Two pastes in the same second get two files.
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), png);
        assert_eq!(second.size, png.len() as u64);

        let _ = std::fs::remove_file(&first.path);
        let _ = std::fs::remove_file(&second.path);
    }

    #[test]
    fn dropped_bytes_land_in_the_inbox_under_a_safe_name() {
        let file = ingest_bytes("rapor.docx", b"PK\x03\x04 not really").unwrap();
        assert!(file.name.starts_with("rapor") && file.name.ends_with(".docx"), "{}", file.name);
        assert!(Path::new(&file.path).starts_with(inbox_dir()));
        assert_eq!(std::fs::read(&file.path).unwrap(), b"PK\x03\x04 not really");

        // A name trying to leave the inbox keeps only its last part.
        let sneaky = ingest_bytes(r"..\..\evil:name?.txt", b"x").unwrap();
        assert!(Path::new(&sneaky.path).starts_with(inbox_dir()));
        assert_eq!(sneaky.name.replace(" (2)", ""), "evil_name_.txt");
        let nameless = ingest_bytes("...", b"y").unwrap();
        assert!(nameless.name.starts_with("file"));

        for f in [&file, &sneaky, &nameless] {
            let _ = std::fs::remove_file(&f.path);
        }
    }
}
