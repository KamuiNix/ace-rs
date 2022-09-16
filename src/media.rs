use anyhow::Result;
use rand::seq::SliceRandom;
use rand::thread_rng;
use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use std::{io::Cursor, path::Path};
use uuid::Uuid;

use crate::anki::Media;

fn with_uuid(prefix: String) -> String {
    let uuid = Uuid::new_v4().to_string();
    return format!("{}-{}", prefix, uuid);
}

async fn fetch_massif(word: &str) -> Result<String> {
    let base_url = String::from("https://massif.la/ja/search?q=");
    let resp = reqwest::get(base_url + word).await?;

    let document = Html::parse_document(&resp.text().await?);
    let selector = Selector::parse("li.text-japanese > div:not(.result-meta)").unwrap();
    let sent_div = document.select(&selector).next().unwrap();
    let sent_text = sent_div.text().collect::<Vec<_>>().join("");

    Ok(sent_text)
}

async fn fetch_chineseboost(word: &str) -> Result<String> {
    let base_url = String::from("https://www.chineseboost.com/chinese-example-sentences?query=");
    let resp = reqwest::get(base_url + word).await?;

    let document = Html::parse_document(&resp.text().await?);
    let selector = Selector::parse(".liju .hanzi.sentence").unwrap();
    let sent_div = document.select(&selector).next().unwrap();
    let sent_text = sent_div.text().collect::<Vec<_>>().join("");

    Ok(sent_text)
}

pub async fn get_sent(word: &str, is_japanese: bool) -> Result<String> {
    if is_japanese {
        fetch_massif(word).await
    } else {
        fetch_chineseboost(word).await
    }
}

async fn download_file(url: &str, output_path: &Path, extension: Option<&str>) -> Result<()> {
    let response = reqwest::get(url).await?;
    let content_type = response.headers().get("Content-Type");

    let mut full_path = output_path.to_path_buf();

    if let Some(ext) = extension {
        full_path.set_extension(ext);
    } else {
        // probe extension from content-type header
        if let Some(ctype) = content_type {
            let ext = ctype.to_str().unwrap().split('/').collect::<Vec<&str>>()[1];
            full_path.set_extension(ext);
        }
    }

    let bytes = response.bytes().await?;
    let mut file = std::fs::File::create(full_path)?;
    let mut content = Cursor::new(bytes);
    std::io::copy(&mut content, &mut file)?;

    Ok(())
}

pub async fn fetch_audio_server(word: &str, custom_audio_server: &str) -> Result<Media> {
    let url = format!("{}{}", custom_audio_server, word);
    let res = reqwest::get(&url).await?.error_for_status()?;
    let filename = with_uuid(word.to_string());

    Ok(Media { url, filename })
}

pub async fn forvo(word: &str) -> Result<Media> {
    let url = format!("https://forvo.com/search/{}/", word);

    let content = reqwest::get(&url).await?.text().await?;

    let regex_sequence_pattern = Regex::new(r"(Play\(\w+,')(\w+=*)").unwrap();
    let code_sequence = regex_sequence_pattern
        .captures_iter(content.as_str())
        .next()
        .ok_or("Could not find forvo pronunciation")
        .unwrap()
        .get(2)
        .unwrap()
        .as_str();
    let url = String::from("https://forvo.com/player-mp3Handler.php?path=") + code_sequence;
    let filename = with_uuid(word.to_string());
    Ok(Media { url, filename })
}

async fn get_fullres_urls(word: &str, is_japanese: bool) -> Result<Vec<String>> {
    let country = if is_japanese { "co.jp" } else { "com.hk" };

    let url = format!("https://google.{}/search?q={}&tbm=isch", country, word);
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Linux; Android 9; SM-G960F Build/PPR1.180610.011; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/74.0.3729.157 Mobile Safari/537.36")
        .build()?;
    let content = client.get(&url).send().await?.text().await?;
    let re = Regex::new(r"AF_initDataCallback\((\{key: 'ds:1'.*?)\);</script>").unwrap();
    let found = re.captures(&content);

    let mut results = vec![];
    if let Some(found) = found {
        let cap = found.get(1);
        if let Some(cap) = cap {
            let json: Value = json5::from_str(cap.as_str()).unwrap();
            let decoded = &json.get("data").unwrap()[31][0][12][2];

            for full_res in decoded.as_array().unwrap() {
                let ent = full_res.get(1);
                if let Some(ent) = ent {
                    let url = &ent[3][0];
                    if !url.is_null() {
                        results.push(url.as_str().unwrap().to_string());
                    }
                }
            }
        }
    }

    Ok(results)
}

pub async fn google_img(word: String, is_japanese: bool) -> Result<Media> {
    let urls = get_fullres_urls(&word, is_japanese).await?;
    let max_offset = if urls.len() < 10 { urls.len() } else { 10 };
    let mut shuffled = urls[..max_offset].to_vec();
    shuffled.shuffle(&mut thread_rng());
    let url = shuffled.get(0).ok_or("Could not find image").unwrap();
    let filename = with_uuid(word.clone());
    Ok(Media {
        url: url.to_string(),
        filename,
    })
}
