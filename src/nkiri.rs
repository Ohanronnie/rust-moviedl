use dialoguer::{Input, Select};
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::thread;
use std::time::Duration;
use url::Url;

fn read_number(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut i = start;
    let mut value: u32 = 0;
    let mut found = false;

    while i < bytes.len() && bytes[i].is_ascii_digit() {
        found = true;
        value = value.saturating_mul(10).saturating_add((bytes[i] - b'0') as u32);
        i += 1;
    }

    if found { Some((value, i)) } else { None }
}

fn extract_episode_number(text: &str) -> Option<u32> {
    // Work on the last path segment to avoid matching random host/path IDs.
    let candidate = if let Ok(parsed) = Url::parse(text) {
        parsed
            .path_segments()
            .and_then(|mut segs| segs.next_back())
            .unwrap_or(text)
            .to_string()
    } else {
        text.to_string()
    };

    let lower = candidate.to_ascii_lowercase();
    let bytes = lower.as_bytes();

    // episode25 / episode-25 / episode 25
    if let Some(idx) = lower.find("episode") {
        let mut i = idx + "episode".len();
        while i < bytes.len() && !bytes[i].is_ascii_digit() {
            i += 1;
        }
        if let Some((num, _)) = read_number(bytes, i) {
            if num > 0 {
                return Some(num);
            }
        }
    }

    // s03e25 / .e25 / _e25
    for i in 0..bytes.len() {
        if bytes[i] == b'e' {
            let prev_ok = if i == 0 {
                false
            } else {
                !bytes[i - 1].is_ascii_alphanumeric()
                    || (bytes[i - 1] == b's' && i >= 2 && bytes[i - 2].is_ascii_digit())
            };
            if prev_ok {
                if let Some((num, end)) = read_number(bytes, i + 1) {
                    let next_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
                    if !next_ok {
                        continue;
                    }
                    if num > 0 {
                        return Some(num);
                    }
                }
            }
        }
    }

    None
}

fn parse_episode_range(input: &str) -> Option<(u32, u32)> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return None;
    }

    if let Some((a, b)) = trimmed.split_once('-') {
        let start = a.trim().parse::<u32>().ok()?;
        let end = b.trim().parse::<u32>().ok()?;
        if start == 0 || end == 0 {
            return Some((0, 0));
        }
        return Some((start.min(end), start.max(end)));
    }

    let n = trimmed.parse::<u32>().ok()?;
    if n == 0 {
        return Some((0, 0));
    }
    Some((n, n))
}

#[cfg(test)]
mod tests {
    use super::{extract_episode_number, parse_episode_range};

    #[test]
    fn extracts_episode_from_filename_pattern() {
        let url = "https://downloadwella.com/k2v5peuya0ge/Jujutsu.Kaisen.E26.(NKIRI.COM).mkv.html";
        assert_eq!(extract_episode_number(url), Some(26));
    }

    #[test]
    fn does_not_extract_from_random_token() {
        let url = "https://downloadwella.com/t82kd6829me1/redirect";
        assert_eq!(extract_episode_number(url), None);
    }

    #[test]
    fn parses_range_inputs() {
        assert_eq!(parse_episode_range("1-24"), Some((1, 24)));
        assert_eq!(parse_episode_range("24-1"), Some((1, 24)));
        assert_eq!(parse_episode_range("0"), Some((0, 0)));
        assert_eq!(parse_episode_range("all"), None);
    }
}

pub fn scrape_nkiri(client: &Client) -> Vec<String> {
    let search_term: String = Input::new()
        .with_prompt("🔍 Enter movie/anime title to search")
        .interact_text()
        .unwrap();

    let search_url = format!(
        "https://thenkiri.com/?s={}&post_type=post",
        urlencoding::encode(&search_term)
    );

    println!("🌐 Searching Thenkiri for '{}'...", search_term);

    let res = client.get(&search_url).send().unwrap().text().unwrap();
    let document = Html::parse_document(&res);

    let article_selector = Selector::parse("article").unwrap();
    let thumb_link_selector = Selector::parse("div.search-entry-inner div.thumbnail a").unwrap();
    let title_link_selector = Selector::parse("h2.search-entry-title a, h2.entry-title a").unwrap();
    let img_selector = Selector::parse("img").unwrap();

    let mut results = vec![];
    let mut seen = HashSet::new();

    for article in document.select(&article_selector) {
        let a_tag = article
            .select(&thumb_link_selector)
            .next()
            .or_else(|| article.select(&title_link_selector).next());

        if let Some(a_tag) = a_tag {
            if let Some(href) = a_tag.value().attr("href") {
                if !href.contains("thenkiri.com/") || !seen.insert(href.to_string()) {
                    continue;
                }

                let name = a_tag
                    .select(&img_selector)
                    .next()
                    .and_then(|img| img.value().attr("alt"))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        a_tag
                            .text()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim()
                            .to_string()
                    });

                if !name.is_empty() {
                    results.push((name, href.to_string()));
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
    let mut final_seen = HashSet::new();
    let mut direct_links = vec![];

    // 1) direct file hosts if present on page
    let link_selector = Selector::parse("a").unwrap();
    for a in document.select(&link_selector) {
        if let Some(href) = a.value().attr("href") {
            if (href.contains("nkiserv.com")
                || href.contains("ds2.nkiserv.com")
                || href.contains("downloadwella.com")
                || href.contains("wetafiles.com"))
                && final_seen.insert(href.to_string())
            {
                direct_links.push(href.to_string());
            }
        }
    }

    // 2) follow elementor buttons through intermediate pages/forms
    let button_selector = Selector::parse("section.elementor-section a.elementor-button").unwrap();
    let mut download_pages: Vec<String> = document
        .select(&button_selector)
        .filter_map(|a| a.value().attr("href"))
        .filter(|href| {
            href.contains("downloadwella.com")
                || href.contains("wetafiles.com")
                || href.contains("nkiserv.com")
        })
        .map(|s| s.to_string())
        .collect();

    let mut episode_pages: Vec<(u32, String)> = download_pages
        .iter()
        .filter_map(|link| extract_episode_number(link).map(|ep| (ep, link.clone())))
        .collect();
    let mut episode_mode = false;

    let likely_episode_post = selected_name.to_ascii_lowercase().contains("episode");
    if episode_pages.len() < 2 && likely_episode_post && download_pages.len() >= 2 {
        episode_mode = true;
        episode_pages = download_pages
            .iter()
            .enumerate()
            .map(|(idx, link)| ((idx as u32) + 1, link.clone()))
            .collect();
        println!(
            "ℹ️ Could not parse explicit episode numbers from links. Using link order as episodes: 1-{}.",
            episode_pages.len()
        );
    }

    if episode_pages.len() >= 2 {
        episode_mode = true;
        episode_pages.sort_by_key(|(ep, _)| *ep);
        episode_pages.dedup_by_key(|(ep, _)| *ep);

        let min_ep = episode_pages.first().map(|(ep, _)| *ep).unwrap_or(1);
        let max_ep = episode_pages.last().map(|(ep, _)| *ep).unwrap_or(min_ep);

        println!(
            "📺 Episodes detected: {} to {} ({} available)",
            min_ep,
            max_ep,
            episode_pages.len()
        );
        println!("ℹ️ If you request more than available, all available episodes will be downloaded.");

        let range_input: String = Input::new()
            .with_prompt("Enter episode range (e.g. 1-24, 1-2), 'all' for all, or 0 to exit")
            .allow_empty(true)
            .interact_text()
            .unwrap();

        let parsed = parse_episode_range(&range_input);
        if let Some((start, end)) = parsed {
            if start == 0 || end == 0 {
                println!("👋 Exiting as requested.");
                return vec![];
            }

            if start < min_ep || end > max_ep || start > max_ep {
                println!(
                    "ℹ️ Requested range is beyond available episodes ({}-{}). Downloading all available episodes.",
                    min_ep, max_ep
                );
                download_pages = episode_pages.into_iter().map(|(_, link)| link).collect();
            } else {
                let selected: Vec<String> = episode_pages
                    .into_iter()
                    .filter(|(ep, _)| *ep >= start && *ep <= end)
                    .map(|(_, link)| link)
                    .collect();

                if selected.is_empty() {
                    println!("❌ No episodes matched that range.");
                    return vec![];
                }

                println!("✅ Selected {} episode link(s).", selected.len());
                download_pages = selected;
            }
        } else {
            download_pages = episode_pages.into_iter().map(|(_, link)| link).collect();
        }
    }

    if !episode_mode {
        final_links.extend(direct_links);
    } else {
        // In episode mode, use only the selected episode pages/resolved links.
        final_seen.clear();
    }

    for page_url in download_pages {
        println!("⏳ Processing redirect: {}", page_url);

        let page_res = match client.get(&page_url).send() {
            Ok(resp) => match resp.text() {
                Ok(body) => body,
                Err(err) => {
                    println!("⚠️ Failed reading host page response for {}: {}", page_url, err);
                    continue;
                }
            },
            Err(err) => {
                println!("⚠️ Failed loading host page {}: {}", page_url, err);
                continue;
            }
        };
        let page_doc = Html::parse_document(&page_res);
        let form_selector = Selector::parse("form[name=\"F1\"] input[type=\"hidden\"]").unwrap();

        let mut form_data = vec![];
        for input in page_doc.select(&form_selector) {
            if let Some(name) = input.value().attr("name") {
                let value = input.value().attr("value").unwrap_or("");
                form_data.push((name, value));
            }
        }

        if form_data.is_empty() {
            if final_seen.insert(page_url.clone()) {
                final_links.push(page_url);
            }
            continue;
        }

        let mut resolved = false;
        for attempt in 1..=3 {
            println!("📡 Submitting download form... (attempt {}/3)", attempt);

            let post_res = match client
                .post(&page_url)
                .form(&form_data)
                .header("Referer", &page_url)
                .send()
            {
                Ok(resp) => resp,
                Err(err) => {
                    println!(
                        "⚠️ Submit failed for {} on attempt {}/3: {}",
                        page_url, attempt, err
                    );
                    if attempt < 3 {
                        thread::sleep(Duration::from_secs(2));
                    }
                    continue;
                }
            };

            if let Some(loc) = post_res.headers().get("location") {
                if let Ok(loc_str) = loc.to_str() {
                    let final_url = if loc_str.starts_with("http") {
                        loc_str.to_string()
                    } else {
                        Url::parse(&page_url)
                            .unwrap()
                            .join(loc_str)
                            .unwrap()
                            .to_string()
                    };

                    if final_seen.insert(final_url.clone()) {
                        println!("✅ Fetched final redirect URL: {}", final_url);
                        final_links.push(final_url);
                    }
                    resolved = true;
                    break;
                }
            }

            let final_url = post_res.url().to_string();
            if final_url != page_url {
                if final_seen.insert(final_url.clone()) {
                    println!("✅ Resolved final URL: {}", final_url);
                    final_links.push(final_url);
                }
                resolved = true;
                break;
            }

            // Some hosts return a confirmation page with a "Start download" link
            // instead of sending an HTTP redirect.
            let body = post_res.text().unwrap_or_default();
            let body_doc = Html::parse_document(&body);
            let body_link_selector = Selector::parse("a[href]").unwrap();
            let maybe_start_link = body_doc
                .select(&body_link_selector)
                .filter_map(|a| {
                    let href = a.value().attr("href")?;
                    let text = a.text().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
                    if text.contains("start download") || href.contains("/d/") {
                        Some(href.to_string())
                    } else {
                        None
                    }
                })
                .next();

            if let Some(start_link) = maybe_start_link {
                let resolved_link = if start_link.starts_with("http") {
                    start_link
                } else {
                    Url::parse(&page_url)
                        .ok()
                        .and_then(|base| base.join(&start_link).ok())
                        .map(|u| u.to_string())
                        .unwrap_or(start_link)
                };

                if resolved_link != page_url && final_seen.insert(resolved_link.clone()) {
                    println!("✅ Resolved final URL from confirmation page: {}", resolved_link);
                    final_links.push(resolved_link);
                    resolved = true;
                    break;
                }
            }

            if attempt < 3 {
                thread::sleep(Duration::from_secs(2));
            }
        }

        if !resolved {
            println!(
                "⚠️ Could not resolve link after 3 attempts (possible captcha/challenge): {}",
                page_url
            );
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
