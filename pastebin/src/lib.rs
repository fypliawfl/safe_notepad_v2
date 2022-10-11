use curl::easy as curl;
use msg::Msg;
use std::{
    fs,
    io::{BufReader, Write},
};
use xml::{reader::XmlEvent, EventReader};

const API_DEV_KEY: &'static str = "Uom1Fo31YrYn0kUq2Owx1QPGRcqMq7Tp";

const API_USER_NAME: &'static str = "arturhelmanau";

const API_USER_PASSWORD: &'static str = "pamqad-dacgus-3Fepti";

fn post(url: &str, data: String, response_capacity: usize) -> anyhow::Result<String> {
    let mut handle = curl::Easy::new();
    handle.url(url)?;
    handle.post(true)?;
    let mut written = false;
    handle.read_function(move |mut bytes| {
        if written {
            Ok(0)
        } else if bytes.write(data.as_bytes()).is_ok() {
            written = true;
            Ok(data.len())
        } else {
            Err(curl::ReadError::Abort)
        }
    })?;
    let mut response_bytes = Vec::with_capacity(response_capacity);
    {
        let mut transfer = handle.transfer();

        transfer
            .write_function(|bytes| {
                response_bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            })
            .unwrap();
        transfer.perform()?;
    }
    let response = String::from_utf8(response_bytes)?;
    if response.starts_with("Bad") {
        anyhow::bail!("{response}");
    } else {
        Ok(response)
    }
}

pub fn api_user_key() -> anyhow::Result<String> {
    if let Ok(api_user_key_bytes) = fs::read("api_user_key.txt") {
        return Ok(String::from_utf8(api_user_key_bytes)?);
    }

    let mut data = String::with_capacity(512);
    data.push_str("api_dev_key=");
    data.push_str(API_DEV_KEY);
    data.push_str("&api_user_name=");
    data.push_str(API_USER_NAME);
    data.push_str("&api_user_password=");
    data.push_str(API_USER_PASSWORD);

    let key = post("https://pastebin.com/api/api_login.php", data, 512)?;

    fs::write("api_user_key.txt", &key)?;
    Ok(key)
}

fn get(api_user_key: &str, api_paste_key: &str) -> anyhow::Result<String> {
    let mut data = String::with_capacity(512);
    data.push_str("api_dev_key=");
    data.push_str(API_DEV_KEY);
    data.push_str("&api_paste_key=");
    data.push_str(api_paste_key);
    data.push_str("&api_option=show_paste&api_user_key=");
    data.push_str(api_user_key);

    post("https://pastebin.com/api/api_post.php", data, 16384)
}

pub type ApiPasteKey = String;

pub fn collect(api_user_key: &str) -> anyhow::Result<Vec<(ApiPasteKey, Msg)>> {
    let mut data = String::with_capacity(256);
    data.push_str("api_dev_key=");
    data.push_str(API_DEV_KEY);
    data.push_str("&api_option=list&api_results_limit=1000&api_user_key=");
    data.push_str(&api_user_key);

    let response = post("https://pastebin.com/api/api_post.php", data, 16384)?;
    if response == "No pastes found." {
        return Ok(vec![]);
    }

    let raw = BufReader::new(response.as_bytes());
    let parser = EventReader::new(raw);
    let mut key_paste = Vec::with_capacity(1000);
    let mut is_key_next = false;
    for event in parser {
        if is_key_next {
            let characters = match event {
                Ok(XmlEvent::Characters(characters)) => characters,
                _ => unreachable!(),
            };
            if let Ok(paste) = serde_json::from_slice(get(api_user_key, &characters)?.as_bytes()) {
                key_paste.push((characters, paste));
            }
            is_key_next = false;
        } else {
            match event {
                Ok(XmlEvent::StartElement { name, .. }) if name.local_name == "paste_key" => {
                    is_key_next = true
                }
                Err(err) => Err(err)?,
                _ => (),
            }
        }
    }
    Ok(key_paste)
}

pub fn remove(api_user_key: &str, api_paste_key: &str) -> anyhow::Result<()> {
    let mut data = String::with_capacity(512);
    data.push_str("api_dev_key=");
    data.push_str(API_DEV_KEY);
    data.push_str("&api_paste_key=");
    data.push_str(api_paste_key);
    data.push_str("&api_option=delete&api_user_key=");
    data.push_str(&api_user_key);

    post("https://pastebin.com/api/api_post.php", data, 256).map(drop)
}

pub fn insert(msg: &Msg) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(msg)?;
    let api_user_key = api_user_key()?;

    let mut data = String::with_capacity(512);
    data.push_str("api_dev_key=");
    data.push_str(API_DEV_KEY);
    data.push_str("&api_paste_code=");
    data.push_str(&String::from_utf8(bytes)?);
    data.push_str("&api_option=paste&api_user_key=");
    data.push_str(&api_user_key);

    post("https://pastebin.com/api/api_post.php", data, 512)
}
