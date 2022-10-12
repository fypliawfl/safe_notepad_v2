use std::io::Write;

use curl::easy::{Easy, List};
use json::{object::Object as JsonObject, JsonValue};
use msg::Msg;
use rand::{thread_rng, Rng};

const AUTHORIZATION_HEADER: &'static str =
    "Authorization: Bearer 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";

const URL: &'static str = "https://api.github.com/gists/8f275b3dfb80dd1ca8e6c46eaa320b86";

pub type FileKey = u128;

pub fn recv(handle: &mut Easy, capacity: usize) -> anyhow::Result<String> {
    let mut response_bytes = Vec::with_capacity(capacity);
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
    Ok(String::from_utf8(response_bytes)?)
}

pub fn handle() -> anyhow::Result<Easy> {
    let mut headers = List::new();
    headers.append("Accept: application/vnd.github+json")?;
    headers.append(AUTHORIZATION_HEADER)?;
    headers.append("User-Agent: Safe Notepad")?;
    let mut handle = Easy::new();
    handle.http_headers(headers)?;
    handle.url(URL)?;
    Ok(handle)
}

fn json_object(json: &JsonValue) -> anyhow::Result<&JsonObject> {
    match json {
        json::JsonValue::Object(object) => Ok(object),
        _ => anyhow::bail!("expected json value to be object"),
    }
}

fn json_object_field<'a>(
    json_object: &'a JsonObject,
    name: &'static str,
) -> anyhow::Result<&'a JsonValue> {
    json_object
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("expected {name} field on json object"))
}

pub fn collect() -> anyhow::Result<Vec<(FileKey, Msg)>> {
    let mut output = Vec::with_capacity(8192);

    let mut handle = handle()?;
    handle.get(true)?;
    let outer_value = json::parse(&recv(&mut handle, 32728)?)?;
    let outer_object = json_object(&outer_value)?;
    let files_value = json_object_field(&outer_object, "files")?;
    let files_object = json_object(&files_value)?;
    for (file_name, file) in files_object.iter() {
        if file_name != "README.md" {
            let file_object = json_object(file)?;
            let file_content = json_object_field(file_object, "content")?
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("expected file {file_name} to have str content"))?;
            let msg = serde_json::from_str(file_content)?;
            output.push((
                (file_name.strip_suffix(".json"))
                    .ok_or_else(|| {
                        anyhow::anyhow!("expected file {file_name} to have json extension")
                    })?
                    .parse()?,
                msg,
            ));
        }
    }

    Ok(output)
}

fn post(handle: &mut Easy, data: String, recv_capaciy: usize) -> anyhow::Result<String> {
    handle.post(true)?;
    let mut written = false;
    handle.read_function(move |mut bytes| {
        if written {
            Ok(0)
        } else if bytes.write(data.as_bytes()).is_ok() {
            written = true;
            Ok(data.len())
        } else {
            Err(curl::easy::ReadError::Abort)
        }
    })?;
    recv(handle, recv_capaciy)
}

pub fn insert(msg: &Msg) -> anyhow::Result<FileKey> {
    let file_key = thread_rng().gen();
    let msg_json_string = serde_json::to_string_pretty(msg)?;
    let post_data = format!(
        "{{ \"files\": {{\"{file_key}.json\": {{ \"content\": \"{msg_json_string}\", \"filename\": \"{file_key}.json\" }} }} }}",
    );

    let mut handle = handle()?;
    post(&mut handle, post_data, 8192)?;

    Ok(file_key)
}

pub fn remove(file_key: FileKey) -> anyhow::Result<()> {
    let post_data = format!("{{ \"files\": {{\"{file_key}.json\": null }} }}",);

    let mut handle = handle()?;
    post(&mut handle, post_data, 8192)?;

    Ok(())
}
