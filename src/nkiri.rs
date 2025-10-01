use dialoguer::{Input, Select}; use reqwest::blocking::Client; use scraper::{Html, Selector}; use std::fs::File; use std::io::Write; use url::Url;

pub fn scrape_nkiri(client: &Client) -> Vec<String> { let search_term: String = Input::new() .with_prompt("🔍 Enter movie/anime title to search") .interact_text() .unwrap();

let search_url = format!(
    "https://nkiri.com/?s={}&post_type=post",
    urlencoding::encode(&search_term)
);

println!("🌐 Searching Nkiri for '{}'...", search_term);

let res = client.get(&search_url).send().unwrap().text().unwrap();
let document = Html::parse_document(&res);

let article_selector = Selector::parse("article").unwrap();
let link_selector = Selector::parse("div.search-entry-inner div.thumbnail a").unwrap();
let img_selector = Selector::parse("img").unwrap();

let mut results = vec![];

for article in document.select(&article_selector) {
    if let Some(a_tag) = article.select(&link_selector).next() {
        if let Some(href) = a_tag.value().attr("href") {
            if let Some(img_tag) = a_tag.select(&img_selector).next() {
                if let Some(alt) = img_tag.value().attr("alt") {
                    results.push((alt.to_string(), href.to_string()));
                }
            }
        }
    }
}

if results.is_empty() {
    println!("❌ No results found.");
    return vec![];
}

let choices: Vec<String> = results
    .iter()
    .enumerate()
    .map(|(i, (name, _))| format!("{}. {}", i + 1, name))
    .collect();

let selection = Select::new()
    .with_prompt("👉 Select which one you want to download")
    .items(&choices)
    .interact()
    .unwrap();

let selected_url = &results[selection].1;
let selected_name = &results[selection].0;

println!("🔗 Fetching download links from: {}", selected_url);

let res = client.get(selected_url).send().unwrap().text().unwrap();
let document = Html::parse_document(&res);

let mut final_links = vec![];

// 1. Check for direct links
let link_selector = Selector::parse("a").unwrap();
for a in document.select(&link_selector) {
    if let Some(href) = a.value().attr("href") {
        if href.contains("nkiserv.com") || href.contains("ds2.nkiserv.com") {
            println!("⚡ Found direct download link: {}", href);
            final_links.push(href.to_string());
        }
    }
}

// 2. Also check indirect (through downloadwella)
let button_selector = Selector::parse("section.elementor-section a.elementor-button").unwrap();
let download_pages: Vec<String> = document
    .select(&button_selector)
    .filter_map(|a| a.value().attr("href"))
    .filter(|href| href.contains("downloadwella.com") || href.contains("wetafiles.com"))
    .map(|s| s.to_string())
    .collect();

for page_url in download_pages {
    println!("⏳ Processing redirect: {}", page_url);

    let page_res = client.get(&page_url).send().unwrap().text().unwrap();
    let page_doc = Html::parse_document(&page_res);
    let form_selector = Selector::parse("form[name=\"F1\"] input[type=\"hidden\"]").unwrap();

    let mut form_data = vec![];
    for input in page_doc.select(&form_selector) {
        if let Some(name) = input.value().attr("name") {
            let value = input.value().attr("value").unwrap_or("");
            form_data.push((name, value));
        }
    }

    println!("📡 Submitting download form...");

    let post_res = client
        .post(&page_url)
        .form(&form_data)
        .header("Referer", &page_url)
        .send()
        .unwrap();

    if let Some(loc) = post_res.headers().get("location") {
        let loc_str = loc.to_str().unwrap();
        let final_url = if loc_str.starts_with("http") {
            loc_str.to_string()
        } else {
            Url::parse(&page_url)
                .unwrap()
                .join(loc_str)
                .unwrap()
                .to_string()
        };

        println!("✅ Fetched final redirect URL: {}", final_url);
        final_links.push(final_url);
    }
}

if final_links.is_empty() {
    println!("❌ No download links were found.");
}

let safe_name = selected_name
    .replace(|c: char| !c.is_alphanumeric(), "_")
    .to_lowercase();
let path = format!("downloads/{}.txt", safe_name);
let mut file = File::create(&path).unwrap();
for link in &final_links {
    writeln!(file, "{}", link).unwrap();
}

println!("📁 Saved {} link(s) to {}", final_links.len(), path);
final_links

}

