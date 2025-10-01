mod downloader;
mod meta;
mod nkiri;

use crate::downloader::download_file;
use crate::nkiri::scrape_nkiri;
use reqwest::blocking::Client;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "Rust Downloader")]
struct Cli {
    #[structopt(name = "urls")]
    urls: Vec<String>,

    #[structopt(long)]
    input: Option<String>,

    #[structopt(long, default_value = "4")]
    chunks: usize,

    #[structopt(long, default_value = "downloads")]
    output: String,

    #[structopt(long)]
    test: bool,

    #[structopt(long, default_value = "nkiri")]
    site: String,
}

fn main() {
    let args = Cli::from_args();
    fs::create_dir_all(&args.output).unwrap();

    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let mut urls = args.urls;

    if let Some(file_path) = &args.input {
        let file = File::open(file_path).expect("Failed to open input file");
        let reader = BufReader::new(file);
        for line in reader.lines() {
            if let Ok(url) = line {
                if !url.trim().is_empty() {
                    urls.push(url.trim().to_string());
                }
            }
        }
    }

    if urls.is_empty() {
        match args.site.as_str() {
            "nkiri" => {
                urls = scrape_nkiri(&client);
            }
            site => {
                println!("❌ Unsupported site: {}", site);
                return;
            }
        }
    }

    for url in urls {
        download_file(&client, &url, args.chunks, &args.output, args.test);
    }
}
