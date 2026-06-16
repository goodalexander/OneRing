use std::sync::Arc;

use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use codex_exec_server::HttpClient;
use codex_exec_server::HttpRequestParams;
use oauth2::AsyncHttpClient;
use oauth2::AuthUrl;
use oauth2::AuthorizationCode;
use oauth2::ClientId;
use oauth2::ClientSecret;
use oauth2::CsrfToken;
use oauth2::EndpointNotSet;
use oauth2::EndpointSet;
use oauth2::HttpRequest;
use oauth2::HttpResponse;
use oauth2::PkceCodeChallenge;
use oauth2::PkceCodeVerifier;
use oauth2::RedirectUrl;
use oauth2::RequestTokenError;
use oauth2::RevocationErrorResponseType;
use oauth2::Scope;
use oauth2::StandardErrorResponse;
use oauth2::StandardRevocableToken;
use oauth2::StandardTokenIntrospectionResponse;
use oauth2::TokenUrl;
use oauth2::basic::BasicErrorResponseType;
use oauth2::basic::BasicTokenType;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderMap;
use rmcp::transport::auth::OAuthTokenResponse;
use rmcp::transport::auth::VendorExtraTokenFields;

use crate::auth_status::StreamableHttpOAuthMetadata;
use crate::utils::protocol_headers;

type OAuthClient = oauth2::Client<
    StandardErrorResponse<BasicErrorResponseType>,
    OAuthTokenResponse,
    StandardTokenIntrospectionResponse<VendorExtraTokenFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

type OAuthClientBuilder = oauth2::Client<
    StandardErrorResponse<BasicErrorResponseType>,
    OAuthTokenResponse,
    StandardTokenIntrospectionResponse<VendorExtraTokenFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
>;

#[derive(serde::Deserialize)]
pub(crate) struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
}

pub(crate) struct OAuthAuthorization {
    client: OAuthClient,
    client_id: String,
    pkce_verifier: PkceCodeVerifier,
    csrf_state: CsrfToken,
    authorization_url: String,
    http_client: RoutedOAuthHttpClient,
}

impl OAuthAuthorization {
    pub(crate) fn new(
        metadata: StreamableHttpOAuthMetadata,
        credentials: OAuthClientCredentials,
        redirect_uri: &str,
        scopes: &[&str],
        default_headers: HeaderMap,
        http_client: Arc<dyn HttpClient>,
    ) -> Result<Self> {
        let OAuthClientCredentials {
            client_id,
            client_secret,
        } = credentials;
        let mut client = OAuthClientBuilder::new(ClientId::new(client_id.clone()))
            .set_auth_uri(AuthUrl::new(metadata.authorization_endpoint)?)
            .set_token_uri(TokenUrl::new(metadata.token_endpoint)?)
            .set_redirect_uri(RedirectUrl::new(redirect_uri.to_string())?);
        if let Some(client_secret) = client_secret {
            client = client.set_client_secret(ClientSecret::new(client_secret));
        }

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);
        for scope in scopes {
            request = request.add_scope(Scope::new((*scope).to_string()));
        }
        let (authorization_url, csrf_state) = request.url();

        Ok(Self {
            client,
            client_id,
            pkce_verifier,
            csrf_state,
            authorization_url: authorization_url.to_string(),
            http_client: RoutedOAuthHttpClient {
                http_client,
                default_headers,
            },
        })
    }

    pub(crate) fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    pub(crate) async fn exchange_code(
        self,
        code: &str,
        csrf_state: &str,
    ) -> Result<(String, OAuthTokenResponse)> {
        if self.csrf_state.secret() != csrf_state {
            bail!("OAuth callback state did not match login request");
        }
        let credentials = match self
            .client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .set_pkce_verifier(self.pkce_verifier)
            .request_async(&self.http_client)
            .await
        {
            Ok(credentials) => credentials,
            Err(RequestTokenError::Parse(_, body)) => {
                serde_json::from_slice::<OAuthTokenResponse>(&body)?
            }
            Err(error) => return Err(anyhow!("OAuth token exchange failed: {error}")),
        };
        Ok((self.client_id, credentials))
    }
}

pub(crate) async fn register_oauth_client(
    metadata: &StreamableHttpOAuthMetadata,
    redirect_uri: &str,
    default_headers: HeaderMap,
    http_client: Arc<dyn HttpClient>,
) -> Result<OAuthClientCredentials> {
    let registration_url = metadata
        .registration_endpoint
        .as_ref()
        .ok_or_else(|| anyhow!("OAuth server does not support dynamic client registration"))?;
    let registration_request = serde_json::json!({
        "client_name": "Codex",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_method": "none",
        "response_types": ["code"],
    });
    let mut headers = default_headers;
    headers.insert(CONTENT_TYPE, "application/json".parse()?);
    let response = http_client
        .http_request(HttpRequestParams {
            method: "POST".to_string(),
            url: registration_url.clone(),
            headers: protocol_headers(&headers),
            body: Some(serde_json::to_vec(&registration_request)?.into()),
            timeout_ms: None,
            request_id: "oauth-register".to_string(),
            stream_response: false,
        })
        .await?;
    if !(200..300).contains(&response.status) {
        bail!(
            "OAuth dynamic client registration returned HTTP {}",
            response.status
        );
    }

    let mut credentials =
        serde_json::from_slice::<OAuthClientCredentials>(&response.body.into_inner())?;
    credentials.client_secret = credentials
        .client_secret
        .filter(|secret| !secret.is_empty());
    Ok(credentials)
}

struct RoutedOAuthHttpClient {
    http_client: Arc<dyn HttpClient>,
    default_headers: HeaderMap,
}

impl<'c> AsyncHttpClient<'c> for RoutedOAuthHttpClient {
    type Error = std::io::Error;
    type Future = futures::future::BoxFuture<'c, Result<HttpResponse, Self::Error>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let mut headers = self.default_headers.clone();
            headers.extend(parts.headers);
            let response = self
                .http_client
                .http_request(HttpRequestParams {
                    method: parts.method.to_string(),
                    url: parts.uri.to_string(),
                    headers: protocol_headers(&headers),
                    body: Some(body.into()),
                    timeout_ms: None,
                    request_id: "oauth-token".to_string(),
                    stream_response: false,
                })
                .await
                .map_err(std::io::Error::other)?;
            let mut builder = oauth2::http::Response::builder().status(response.status);
            for header in response.headers {
                builder = builder.header(header.name, header.value);
            }
            builder
                .body(response.body.into_inner())
                .map_err(std::io::Error::other)
        })
    }
}
