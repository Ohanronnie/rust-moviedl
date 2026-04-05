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

pub fn download_file(client: &Client, url: &str, chunks: usize, output_dir: &str, test: bool) {
    let filename = url.split('/').last().unwrap_or("file.bin");
    let full_path = format!("{}/{}", output_dir, filename);

    if test {
        println!("\n🧪 Test mode: skipping download for {}", filename);
        return;
    }

    let size = get_file_size(client, url);
    if size == 0 {
        eprintln!("❌ Could not determine file size for URL, skipping: {}", url);
        return;
    }

    println!("\n📦 File: {}", filename);
    println!("📏 Size: {:.2} MB", size as f64 / 1024.0 / 1024.0);

    let meta_path = full_path.clone();
    let meta = load_meta(&meta_path).unwrap_or_else(|| MetaData {
        url: url.to_string(),
        file: filename.to_string(),
        size,
        chunk_size: (size + chunks as u64 - 1) / chunks as u64,
        chunks,
        progress: vec![false; chunks],
    });

    let chunk_size = meta.chunk_size;
    let chunks = meta.chunks;
    let meta_arc = Arc::new(Mutex::new(meta));

    let mp = Arc::new(MultiProgress::new());
    let mut handles = vec![];
    let mut parts = vec![];

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

        handles.push(thread::spawn(move || {
            download_chunk(client, url, start, end, path, pb_clone, i, meta_path_clone, meta_clone);
        }));
    }

    mp.clear().unwrap();
    for h in handles {
        h.join().unwrap();
    }

    let meta = meta_arc.lock().unwrap();
    if meta.progress.iter().all(|&b| b) {
        let parts: Vec<String> = (0..chunks)
            .map(|i| format!("{}.part{}", full_path, i))
            .collect();
        merge_chunks(&full_path, &parts);
        println!("✅ Done: {}", full_path);
    } else {
        println!("⏸️ Not all chunks downloaded. Will resume next time.");
    }
}

fn get_file_size(client: &Client, url: &str) -> u64 {
    for _ in 0..10 {
        if let Ok(resp) = client.head(url).send() {
            if let Some(size) = resp.headers().get(header::CONTENT_LENGTH) {
                if let Ok(size) = size.to_str().unwrap_or("0").parse::<u64>() {
                    if size > 0 {
                        return size;
                    }
                }
            }
        }

        if let Ok(resp) = client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .send()
        {
            if let Some(range) = resp.headers().get(header::CONTENT_RANGE) {
                if let Ok(range) = range.to_str() {
                    if let Some((_, total)) = range.rsplit_once('/') {
                        if let Ok(total) = total.parse::<u64>() {
                            if total > 0 {
                                return total;
                            }
                        }
                    }
                }
            }
        }
        println!("Retrying HEAD request...");
        thread::sleep(std::time::Duration::from_secs(2));
    }
    0
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
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("Can't open part file");

    let mut downloaded = file.metadata().unwrap().len();
    file.seek(SeekFrom::Start(downloaded)).unwrap();
    let total = end - start + 1;

    while downloaded < total {
        let range_start = start + downloaded;
        let range = format!("bytes={}-{}", range_start, end);

        let res = client.get(&url).header(header::RANGE, range).send();

        match res {
            Ok(mut res) => {
                let mut buf = [0u8; 8192];
                let mut last = Instant::now();
                let mut last_bytes = downloaded;

                while let Ok(n) = res.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    file.write_all(&buf[..n]).unwrap();
                    downloaded += n as u64;
                    bar.set_position(downloaded);

                    if last.elapsed().as_secs_f32() >= 1.0 {
                        let speed =
                            (downloaded - last_bytes) as f32 / last.elapsed().as_secs_f32();
                        bar.set_message(format!("{:.1} KB/s", speed / 1024.0));
                        last = Instant::now();
                        last_bytes = downloaded;
                    }
                }

                if downloaded >= total {
                    let mut m = meta.lock().unwrap();
                    m.progress[chunk_index] = true;
                    let _ = save_meta(&meta_path, &m);
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

    let mut out = File::create(filename).unwrap();
    for part in parts {
        let mut file = File::open(part).unwrap();
        std::io::copy(&mut file, &mut out).unwrap();
        fs::remove_file(part).unwrap();
    }
}
