//! Google sign-in through OpenID Connect: Authorization Code flow with PKCE, state and nonce.
//! ID token signature, issuer, audience, expiry and nonce are verified by the `openidconnect`
//! crate against Google's published discovery document and keys.

use anyhow::{anyhow, Context, Result};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use super::VerifiedIdentity;
use crate::config::{Config, GoogleConfig};

pub struct GoogleOidc {
    metadata: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_url: RedirectUrl,
    http: reqwest::Client,
}

pub struct AuthorizationRequest {
    pub url: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

impl GoogleOidc {
    pub async fn discover(google: &GoogleConfig, config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            // Following redirects from the token endpoint would be an SSRF vector.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let metadata = CoreProviderMetadata::discover_async(IssuerUrl::new(google.issuer_url.clone())?, &http).await?;
        Ok(GoogleOidc {
            metadata,
            client_id: ClientId::new(google.client_id.clone()),
            client_secret: ClientSecret::new(google.client_secret.clone()),
            redirect_url: RedirectUrl::new(config.app_url("/api/v1/auth/google/callback"))?,
            http,
        })
    }

    fn client(
        &self,
    ) -> CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    > {
        CoreClient::from_provider_metadata(self.metadata.clone(), self.client_id.clone(), Some(self.client_secret.clone()))
            .set_redirect_uri(self.redirect_url.clone())
    }

    pub fn authorize(&self) -> AuthorizationRequest {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client()
            .authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        AuthorizationRequest { url: url.to_string(), state: state.secret().clone(), nonce: nonce.secret().clone(), pkce_verifier: verifier.secret().clone() }
    }

    pub async fn exchange(&self, code: String, pkce_verifier: String, nonce: String) -> Result<VerifiedIdentity> {
        let client = self.client();
        let token = client
            .exchange_code(AuthorizationCode::new(code))
            .context("token endpoint missing from discovery document")?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(|e| anyhow!("token exchange failed: {e}"))?;
        let id_token = token.id_token().ok_or_else(|| anyhow!("Google did not return an ID token"))?;
        let claims = id_token.claims(&client.id_token_verifier(), &Nonce::new(nonce)).map_err(|e| anyhow!("ID token verification failed: {e}"))?;
        Ok(VerifiedIdentity {
            provider: "google".into(),
            subject: claims.subject().as_str().to_string(),
            email: claims.email().map(|e| e.as_str().to_string()).ok_or_else(|| anyhow!("ID token has no email claim"))?,
            email_verified: claims.email_verified().unwrap_or(false),
            name: claims.name().and_then(|n| n.get(None)).map(|n| n.as_str().to_string()),
            picture: claims.picture().and_then(|p| p.get(None)).map(|p| p.as_str().to_string()),
        })
    }
}
