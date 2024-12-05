use crate::errors::RequestError;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};

#[derive(Clone, Debug)]
pub struct Action(String);

impl Action {
    pub fn new(action: &str) -> Action {
        Action(action.into())
    }
}

const HEADER_NAME: &str = "SOAPAction";

pub async fn send_async(url: &str, action: Action, body: &str) -> Result<String, RequestError> {
    let client = reqwest::Client::new();

    let resp = client
        .post(url)
        .header(HEADER_NAME, action.0)
        .header(CONTENT_TYPE, "text/xml")
        .header(CONTENT_LENGTH, body.len() as u64)
        .body(body.to_owned())
        .send()
        .await?;

    Ok(resp.text().await?)
}
