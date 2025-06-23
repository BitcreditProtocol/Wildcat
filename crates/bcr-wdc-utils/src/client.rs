// ----- standard library imports
use std::sync::{Arc, RwLock};
// ----- extra library imports
use reqwest::{Result, Url};
// ----- local imports

// ----- end imports

#[derive(Debug, Clone, Default)]
pub struct AuthorizationPlugin {
    token: Arc<RwLock<Option<String>>>,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: String,
}

impl AuthorizationPlugin {
    pub async fn authenticate(
        &mut self,
        client: reqwest::Client,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        let resp: TokenResponse = client
            .post(token_url.clone())
            .form(&[
                ("grant_type", "password"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("username", username),
                ("password", password),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let TokenResponse {
            access_token,
            expires_in,
            refresh_token,
            ..
        } = resp;
        {
            let mut token_lock = self.token.write().unwrap();
            *token_lock = Some(access_token);
        }
        let expiration =
            std::time::Duration::from_secs(expires_in) - std::time::Duration::from_secs(5);

        let token_recipient = Arc::clone(&self.token);
        let cloned_client_id = client_id.to_string();
        tokio::task::spawn(async move {
            tokio::time::sleep(expiration).await;
            let cloned_recipient = Arc::clone(&token_recipient);
            let (expiration, refresh_token) = refresh_access_token(
                client.clone(),
                token_url.clone(),
                refresh_token,
                cloned_client_id.clone(),
                cloned_recipient,
            )
            .await;
            tokio::task::spawn(async move {
                tokio::time::sleep(expiration).await;
                refresh_n_repeat(
                    client,
                    token_url,
                    refresh_token,
                    cloned_client_id,
                    token_recipient,
                )
                .await;
            });
        });
        Ok(())
    }

    pub fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let locked = self.token.read().unwrap();
        if let Some(token) = locked.as_ref() {
            request.bearer_auth(token)
        } else {
            request
        }
    }
}

async fn refresh_n_repeat(
    cl: reqwest::Client,
    token_url: Url,
    mut refresh_token: String,
    client_id: String,
    token_recipient: Arc<RwLock<Option<String>>>,
) {
    loop {
        let cloned_recipient = Arc::clone(&token_recipient);
        let (expiration, new_refresh_token) = refresh_access_token(
            cl.clone(),
            token_url.clone(),
            refresh_token.clone(),
            client_id.clone(),
            cloned_recipient,
        )
        .await;
        refresh_token = new_refresh_token;

        tokio::time::sleep(expiration).await;
    }
}

async fn refresh_access_token(
    cl: reqwest::Client,
    token_url: Url,
    refresh_token: String,
    client_id: String,
    token_recipient: Arc<RwLock<Option<String>>>,
) -> (std::time::Duration, String) {
    let request = cl.post(token_url).form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh_token),
        ("client_id", &client_id),
    ]);
    let response = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to refresh access token: {}", e);
            let expiration = std::time::Duration::from_secs(1);
            return (expiration, refresh_token);
        }
    };
    let token = match response.json::<TokenResponse>().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to parse token response: {}", e);
            let expiration = std::time::Duration::from_secs(1);
            return (expiration, refresh_token);
        }
    };
    let TokenResponse {
        access_token,
        expires_in,
        refresh_token: new_refresh_token,
        ..
    } = token;

    {
        let mut locked = token_recipient.write().unwrap();
        locked.replace(access_token);
    }

    let expiration = std::time::Duration::from_secs(expires_in) - std::time::Duration::from_secs(5);
    (expiration, new_refresh_token)
}
