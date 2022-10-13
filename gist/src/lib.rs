use std::io::Write;

use curl::easy::{Easy, List};
use json::{object::Object as JsonObject, JsonValue};
use msg::Msg;

pub type GistId = String;

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
    let response_string = String::from_utf8(response_bytes)?;
    let response_code = handle.response_code()?;
    if ![200, 201, 204].contains(&response_code) {
        anyhow::bail!(
            "recv failed with response code {response_code} and response string {response_string}"
        );
    } else {
        Ok(response_string)
    }
}

pub fn handle(url: &str) -> anyhow::Result<Easy> {
    let mut headers = List::new();
    headers.append("Accept: application/vnd.github+json")?;
    headers.append("Authorization: Bearer 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")?;
    headers.append("User-Agent: Safe Notepad")?;
    let mut handle = Easy::new();
    handle.http_headers(headers)?;
    handle.url(url)?;
    Ok(handle)
}

fn json_array(json: &JsonValue) -> anyhow::Result<&json::Array> {
    match json {
        json::JsonValue::Array(array) => Ok(array),
        _ => anyhow::bail!("expected json value to be an array"),
    }
}

fn json_object(json: &JsonValue) -> anyhow::Result<&JsonObject> {
    match json {
        json::JsonValue::Object(object) => Ok(object),
        _ => anyhow::bail!("expected json value to be an object"),
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

fn gist_id(gist_info_value: &JsonValue) -> anyhow::Result<GistId> {
    let gist_info_object = json_object(&gist_info_value)?;
    let gist_id = json_object_field(&gist_info_object, "id")?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("expected gist info to contain id string"))?;
    Ok(gist_id.into())
}

pub fn collect() -> anyhow::Result<Vec<(GistId, Msg)>> {
    let mut output = Vec::with_capacity(256);

    let mut handle = handle("https://api.github.com/gists")?;
    handle.get(true)?;
    let gists_value = json::parse(&recv(&mut handle, 32728)?)?;
    let gists_array = json_array(&gists_value)?;
    for gist_info in gists_array {
        let gist_id = gist_id(&gist_info)?;
        handle.url(&format!("https://api.github.com/gists/{gist_id}"))?;
        let gist_value = json::parse(&recv(&mut handle, 16384)?)?;
        let gist_object = json_object(&gist_value)?;
        let files_value = json_object_field(&gist_object, "files")?;
        let files_object = json_object(&files_value)?;
        if let Some((_, file)) = files_object
            .iter()
            .find(|(file_name, _)| *file_name == "msg.json")
        {
            let file_object = json_object(file)?;
            let file_content = json_object_field(file_object, "content")?
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("expected file msg.json to have string content"))?;
            let msg = serde_json::from_str(file_content)?;
            output.push((gist_id.into(), msg));
        }
    }

    Ok(output)
}

pub fn insert(msg: &Msg) -> anyhow::Result<GistId> {
    let msg_json_string = serde_json::to_string_pretty(msg)?
        .replace('\n', "\\n")
        .replace('\"', "\\\"");
    let data =
        format!("{{ \"description\": \"Safe Notepad Msg\", \"public\": true, \"files\": {{\"msg.json\": {{ \"content\": \"{msg_json_string}\" }} }} }}");
    std::fs::write("data.json", &data)?;

    let mut handle = handle("https://api.github.com/gists")?;
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
    handle.post(true)?;
    let gist_info_value = json::parse(&recv(&mut handle, 8192)?)?;

    Ok(gist_id(&gist_info_value)?)
}

pub fn remove(gist_id: &str) -> anyhow::Result<()> {
    let mut handle = handle(&format!("https://api.github.com/gists/{gist_id}"))?;
    handle.custom_request("DELETE")?;
    recv(&mut handle, 64)?;
    Ok(())
}
