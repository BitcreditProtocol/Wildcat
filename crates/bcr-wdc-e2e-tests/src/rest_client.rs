// ----- standard library imports
// ----- extra library imports
use anyhow::Result;
use reqwest::Client as HttpClient;
use reqwest::Url;
use serde::{de::DeserializeOwned, Serialize};
// ----- local modules
// ----- end imports

pub struct RestClient {
    http: HttpClient,
}

impl RestClient {
    pub fn new() -> Self {
        let http = HttpClient::builder().build().unwrap();
        RestClient { http }
    }

    pub async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let resp = self.http.get(url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post<Req: Serialize, Res: DeserializeOwned>(
        &self,
        url: Url,
        body: &Req,
    ) -> Result<Res> {
        let resp = self
            .http
            .post(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post_<Req: Serialize>(&self, url: Url, body: &Req) -> Result<()> {
        self.http
            .post(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
