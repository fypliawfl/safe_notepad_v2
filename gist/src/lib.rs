use std::io::Write;

use curl::easy::{Easy, List};
use json::{object::Object as JsonObject, JsonValue};
use msg::Msg;
use rand::{thread_rng, Rng};

// -X, --request <command>
// (HTTP) Specifies a custom request method to use when communicating with the HTTP server.  The specified request method will be used instead of
// the method otherwise used (which defaults to GET). Read the HTTP 1.1 specification for details and explanations. Common additional HTTP
// requests include PUT and DELETE, but related technologies like WebDAV offers PROPFIND, COPY, MOVE and more.

// Normally you don't need this option. All sorts of GET, HEAD, POST and PUT requests are rather invoked by using dedicated command line options.

// This option only changes the actual word used in the HTTP request, it does not alter the way curl behaves. So for example if you want to make a
// proper HEAD request, using -X HEAD will not suffice. You need to use the -I, --head option.

// The method string you set with -X, --request will be used for all requests, which if you for example use -L, --location may cause unintended
// side-effects when curl doesn't change request method according to the HTTP 30x response codes - and similar.

// (FTP) Specifies a custom FTP command to use instead of LIST when doing file lists with FTP.

// (POP3) Specifies a custom POP3 command to use instead of LIST or RETR. (Added in 7.26.0)

// (IMAP) Specifies a custom IMAP command to use instead of LIST. (Added in 7.30.0)

// (SMTP) Specifies a custom SMTP command to use instead of HELP or VRFY. (Added in 7.34.0)

// If this option is used several times, the last one will be used.

// Examples:
//  curl -X "DELETE" https://example.com
//  curl -X NLST ftp://example.com/

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
        let mut written = false;
        transfer.read_function(move |mut bytes| {
            dbg!();
            if written {
                Ok(0)
            } else if bytes.write(b"{{ \"files\": {{\"README.md\": {{ \"content\": \"123\" }}}}}}").is_ok() {
                written = true;
                Ok(b"{{ \"files\": {{\"README.md\": {{ \"content\": \"123\" }}}}}}".len())
            } else {
                Err(curl::easy::ReadError::Abort)
            }
        })?;
        transfer.perform()?;
    }
    match handle.response_code()? {
        304 => anyhow::bail!("not modified"),
        403 => anyhow::bail!("forbidden gist"),
        404 => anyhow::bail!("resource not found"),
        422 => anyhow::bail!("validation failed, or the endpoint has been spammed"),
        200 => Ok(String::from_utf8(response_bytes)?),
        _ => unreachable!(),
    }
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

fn patch(handle: &mut Easy, data: String, recv_capaciy: usize) -> anyhow::Result<String> {
    handle.custom_request("PATCH")?;
    recv(handle, recv_capaciy)
}

pub fn insert(msg: &Msg) -> anyhow::Result<FileKey> {
    let file_key = thread_rng().gen();
    let msg_json_string = serde_json::to_string_pretty(msg)?;
    let post_data = format!(
        // "{{ \"files\": {{\"{file_key}.json\": {{ \"content\": \"{msg_json_string}\", \"filename\": \"{file_key}.json\" }} }} }}",
        "{{ \"files\": {{\"README.md\": {{ \"content\": \"123\" }}}}}}",
    );

    std::fs::write("post_data.json", &post_data)?;

    let mut handle = handle()?;
    std::fs::write("patch.json", &patch(&mut handle, post_data, 8192)?)?;

    Ok(file_key)
}

pub fn remove(file_key: FileKey) -> anyhow::Result<()> {
    let post_data = format!("{{ \"files\": {{\"{file_key}.json\": null }} }}",);

    let mut handle = handle()?;
    patch(&mut handle, post_data, 8192)?;

    Ok(())
}
