use crate::meta::{load_meta, save_meta, MetaData};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::{blocking::Client, header};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use threadpool::ThreadPool;
use num_cpus;

// Constants for optimization
const MIN_CHUNK_SIZE: u64 = 10 * 1024 * 1024; // 10MB minimum chunk size
const MAX_CONCURRENT_DOWNLOADS: usize = 32; // Maximum concurrent downloads
const BUFFER_SIZE: usize = 256 * 1024; // 256KB buffer size
const PROGRESS_UPDATE_INTERVAL: f32 = 2.0; // Update progress every 2 seconds

pub fn download_file(client: &Client, url: &str, chunks: usize, output_dir: &str, test: bool) {
    let size = get_file_size(client, url);
    let filename = url.split('/').last().unwrap_or("file.bin");
    let full_path = format!("{}/{}", output_dir, filename);

    println!("\n📦 File: {}", filename);
    println!("📏 Size: {:.2} MB", size as f64 / 1024.0 / 1024.0);

    if test {
        return;
    }

    // Calculate optimal chunk count
    let chunks = calculate_optimal_chunks(size, chunks);
    let chunk_size = (size + chunks as u64 - 1) / chunks as u64;

    let meta_path = full_path.clone();
    let meta = load_meta(&meta_path).unwrap_or_else(|| MetaData {
        url: url.to_string(),
        file: filename.to_string(),
        size,
        chunk_size,
        chunks,
        progress: vec![false; chunks],
    });

    let meta_arc = Arc::new(Mutex::new(meta));
    let mp = Arc::new(MultiProgress::new());
    let pool = ThreadPool::new(std::cmp::min(
        num_cpus::get() * 2,
        MAX_CONCURRENT_DOWNLOADS,
    ));

    let mut parts = Vec::with_capacity(chunks);

    for i in 0..chunks {
        let meta_locked = meta_arc.lock().unwrap();
        if meta_locked.progress[i] {
            println!("✅ Chunk {} already downloaded, skipping.", i);
            continue;
        }
        drop(meta_locked);

        let start = i as u64 * chunk_size;
        let end = std::cmp::min(size - 1, (i as u64 + 1) * chunk_size - 1);
        let part_path = format!("{}.part{}", full_path, i);
        parts.push(part_path.clone());

        let pb = mp.add(ProgressBar::new(end - start + 1));
        pb.set_style(
            ProgressStyle::with_template("[{bar:40.cyan/blue}] {bytes}/{total_bytes} {msg}")
                .unwrap(),
        );

        let client = client.clone();
        let url = url.to_string();
        let pb_clone = pb.clone();
        let path = part_path.clone();
        let meta_path_clone = meta_path.clone();
        let meta_clone = Arc::clone(&meta_arc);

        pool.execute(move || {
            download_chunk(
                client, url, start, end, path, pb_clone, i, meta_path_clone, meta_clone,
            );
        });
    }

    pool.join();
    mp.clear().unwrap();

    let meta = meta_arc.lock().unwrap();
    if meta.progress.iter().all(|&b| b) {
        merge_chunks(&full_path, &parts);
        println!("✅ Done: {}", full_path);
    } else {
        println!("⏸️ Not all chunks downloaded. Will resume next time.");
    }
}

fn calculate_optimal_chunks(file_size: u64, requested_chunks: usize) -> usize {
    let cores = num_cpus::get();
    
    // Calculate based on file size (minimum 10MB per chunk)
    let size_based = (file_size / MIN_CHUNK_SIZE).max(1) as usize;
    
    // Calculate based on CPU cores (2 chunks per core)
    let core_based = cores * 2;
    
    // Use the maximum of requested chunks, size-based, and core-based
    requested_chunks.max(size_based).max(core_based)
        .min(MAX_CONCURRENT_DOWNLOADS) // But don't exceed our maximum
}

fn get_file_size(client: &Client, url: &str) -> u64 {
    loop {
        match client.head(url).send() {
            Ok(resp) => {
                if let Some(size) = resp.headers().get(header::CONTENT_LENGTH) {
                    if let Ok(size) = size.to_str().unwrap_or("0").parse::<u64>() {
                        return size;
                    }
                }
            }
            Err(e) => {
                eprintln!("🌐 HEAD request failed: {}, retrying in 3s...", e);
                thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

fn download_chunk(
    client: Client,
    url: String,
    start: u64,
    end: u64,
    path: String,
    bar: ProgressBar,
    chunk_index: usize,
    meta_path: String,
    meta: Arc<Mutex<MetaData>>,
) {
    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("❌ Failed to open part file: {}", e);
            return;
        }
    };

    let downloaded = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    if downloaded > 0 {
        file.seek(SeekFrom::Start(downloaded)).unwrap();
    }

    let total = end - start + 1;
    let mut buf = vec![0u8; BUFFER_SIZE];
    let mut last_update = Instant::now();
    let mut last_bytes = downloaded;

    while downloaded < total {
        let range_start = start + downloaded;
        let range = format!("bytes={}-{}", range_start, end);

        match client.get(&url).header(header::RANGE, range).send() {
            Ok(mut res) => {
                while let Ok(n) = res.read(&mut buf) {
                    if n == 0 {
                        break;
                    }

                    if let Err(e) = file.write_all(&buf[..n]) {
                        eprintln!("❌ Write error: {}, retrying...", e);
                        thread::sleep(std::time::Duration::from_secs(1));
                        break;
                    }

                    let new_downloaded = downloaded + n as u64;
                    bar.set_position(new_downloaded);

                    if last_update.elapsed().as_secs_f32() >= PROGRESS_UPDATE_INTERVAL {
                        let speed = (new_downloaded - last_bytes) as f32
                            / last_update.elapsed().as_secs_f32();
                        bar.set_message(format!("{:.1} KB/s", speed / 1024.0));
                        last_update = Instant::now();
                        last_bytes = new_downloaded;
                    }
                }

                if downloaded >= total {
                    let mut m = meta.lock().unwrap();
                    m.progress[chunk_index] = true;
                    if let Err(e) = save_meta(&meta_path, &m) {
                        eprintln!("⚠️ Failed to save metadata: {}", e);
                    }
                }

                break;
            }
            Err(e) => {
                eprintln!("🌐 Network error: {}, retrying in 3s...", e);
                thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }

    bar.finish_with_message("✅ Done");
}

fn merge_chunks(filename: &str, parts: &[String]) {
    if parts.iter().any(|p| !Path::new(p).exists()) {
        println!("⚠️ Skipping merge. Some chunks are missing.");
        return;
    }

    let mut out = match File::create(filename) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("❌ Failed to create output file: {}", e);
            return;
        }
    };

    for part in parts {
        match File::open(part) {
            Ok(mut file) => {
                if let Err(e) = std::io::copy(&mut file, &mut out) {
                    eprintln!("❌ Failed to merge chunk {}: {}", part, e);
                    return;
                }
                if let Err(e) = fs::remove_file(part) {
                    eprintln!("⚠️ Failed to remove chunk {}: {}", part, e);
                }
            }
            Err(e) => {
                eprintln!("❌ Failed to open chunk {}: {}", part, e);
                return;
            }
        }
    }
}
