use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use flate2::read::GzDecoder;
use log::{error, info};
use tar::Archive;

use crate::settings::TarExtractor;

pub fn start_tar_extractors(
    extractors: Vec<TarExtractor>,
    scan_interval: u64,
    stop_flag: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            for extractor in &extractors {
                if let Err(e) = extract_archives(&extractor.directory) {
                    error!(
                        "Error scanning tar extractor directory '{}': {}",
                        extractor.directory.display(),
                        e
                    );
                }
            }
            thread::sleep(Duration::from_millis(scan_interval));
        }
    })
}

fn extract_archives(directory: &Path) -> io::Result<()> {
    let mut archives = Vec::new();
    find_archives(directory, &mut archives)?;

    for path in archives {
        info!("Extracting {}", path.display());
        if let Err(e) = extract_tar_archive(&path) {
            error!("Error extracting '{}': {}", path.display(), e);
        }
    }

    Ok(())
}

fn find_archives(directory: &Path, archives: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            find_archives(&path, archives)?;
        } else if file_type.is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".tar.gz"))
        {
            archives.push(path);
        }
    }
    Ok(())
}

fn extract_tar_archive(path: &Path) -> io::Result<()> {
    let file = File::open(path)?;
    let mut archive = Archive::new(GzDecoder::new(file));
    archive.unpack(path.parent().unwrap_or_else(|| Path::new(".")))?;
    fs::remove_file(path)
}
