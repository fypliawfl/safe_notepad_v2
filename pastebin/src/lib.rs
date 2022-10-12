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

const API_USER_KEY_FILE_NAME: &'static str = "api_user_key.txt";

#[cfg(feature = "mock")]
const MOCK_PASTEBIN_FILE_NAME: &'static str = "mock_pastebin.json";

pub fn api_user_key() -> anyhow::Result<String> {
    #[cfg(feature = "mock")]
    if fs::File::open(MOCK_PASTEBIN_FILE_NAME).is_err() {
        fs::File::create(MOCK_PASTEBIN_FILE_NAME)?;
    }

    if let Ok(api_user_key_bytes) = fs::read(API_USER_KEY_FILE_NAME) {
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

    fs::write(API_USER_KEY_FILE_NAME, &key)?;

    Ok(key)
}

#[cfg(not(feature = "mock"))]
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
    #[cfg(not(feature = "mock"))]
    {
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
        let mut key_msgs = Vec::with_capacity(1000);
        let mut is_key_next = false;
        for event in parser {
            if is_key_next {
                let characters = match event {
                    Ok(XmlEvent::Characters(characters)) => characters,
                    _ => unreachable!(),
                };
                if let Ok(msg) = serde_json::from_slice(get(api_user_key, &characters)?.as_bytes())
                {
                    key_msgs.push((characters, msg));
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
        Ok(key_msgs)
    }
    #[cfg(feature = "mock")]
    {
        let bytes = fs::read(MOCK_PASTEBIN_FILE_NAME)?;
        if bytes.is_empty() {
            Ok(vec![])
        } else {
            Ok(serde_json::from_slice(&bytes)?)
        }
    }
}

pub fn remove(api_user_key: &str, api_paste_key: &str) -> anyhow::Result<()> {
    #[cfg(not(feature = "mock"))]
    {
        let mut data = String::with_capacity(512);
        data.push_str("api_dev_key=");
        data.push_str(API_DEV_KEY);
        data.push_str("&api_paste_key=");
        data.push_str(api_paste_key);
        data.push_str("&api_option=delete&api_user_key=");
        data.push_str(&api_user_key);

        post("https://pastebin.com/api/api_post.php", data, 256).map(drop)
    }
    #[cfg(feature = "mock")]
    {
        let mut key_msgs = collect(api_user_key)?;
        if let Some(position) = key_msgs
            .iter()
            .position(|(other_api_paste_key, _)| other_api_paste_key == api_paste_key)
        {
            key_msgs.remove(position);
            fs::write(MOCK_PASTEBIN_FILE_NAME, &serde_json::to_vec(&key_msgs)?)?;
        }
        Ok(())
    }
}

pub fn insert(api_user_key: &str, msg: &Msg) -> anyhow::Result<String> {
    #[cfg(not(feature = "mock"))]
    {
        let bytes = serde_json::to_vec(msg)?;

        let mut data = String::with_capacity(512);
        data.push_str("api_dev_key=");
        data.push_str(API_DEV_KEY);
        data.push_str("&api_paste_code=");
        data.push_str(&String::from_utf8(bytes)?);
        data.push_str("&api_option=paste&api_user_key=");
        data.push_str(api_user_key);

        post("https://pastebin.com/api/api_post.php", data, 512)
    }

    #[cfg(feature = "mock")]
    {
        let mut key_msgs = collect(api_user_key)?;
        let api_paste_key = rand::Rng::gen::<u128>(&mut rand::thread_rng()).to_string();
        key_msgs.push((api_paste_key.clone(), msg.clone()));
        fs::write(MOCK_PASTEBIN_FILE_NAME, &serde_json::to_vec_pretty(&key_msgs)?)?;
        Ok(api_paste_key)
    }
}
