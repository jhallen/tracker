//! OAuth 2.0 / OpenID Connect login ("Sign in with Google" etc.).
//!
//! Every provider uses the authorization-code flow with PKCE:
//!
//! 1. `GET /auth/{provider}/start` stores `state` + PKCE verifier in a short
//!    lived HttpOnly cookie and redirects the browser to the provider.
//! 2. The provider redirects back to `GET /auth/{provider}/callback?code&state`.
//! 3. We check `state`, exchange `code` for an access token, fetch the user
//!    profile and map it to an [`ExternalProfile`].
//!
//! A provider is enabled when its `<PREFIX>_CLIENT_ID` and
//! `<PREFIX>_CLIENT_SECRET` environment variables are set.

use std::env;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Google,
    Microsoft,
    GitHub,
    GitLab,
    Facebook,
    Discord,
    /// Any standards-compliant OpenID Connect provider (Okta, Auth0,
    /// Keycloak, Authentik, Azure AD B2C, ...), configured with explicit URLs.
    Oidc,
}

impl ProviderKind {
    pub const ALL: &'static [ProviderKind] = &[
        ProviderKind::Google,
        ProviderKind::Microsoft,
        ProviderKind::GitHub,
        ProviderKind::GitLab,
        ProviderKind::Facebook,
        ProviderKind::Discord,
        ProviderKind::Oidc,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            ProviderKind::Google => "google",
            ProviderKind::Microsoft => "microsoft",
            ProviderKind::GitHub => "github",
            ProviderKind::GitLab => "gitlab",
            ProviderKind::Facebook => "facebook",
            ProviderKind::Discord => "discord",
            ProviderKind::Oidc => "oidc",
        }
    }

    fn env_prefix(self) -> &'static str {
        match self {
            ProviderKind::Google => "GOOGLE",
            ProviderKind::Microsoft => "MICROSOFT",
            ProviderKind::GitHub => "GITHUB",
            ProviderKind::GitLab => "GITLAB",
            ProviderKind::Facebook => "FACEBOOK",
            ProviderKind::Discord => "DISCORD",
            ProviderKind::Oidc => "OIDC",
        }
    }

    fn default_name(self) -> &'static str {
        match self {
            ProviderKind::Google => "Google",
            ProviderKind::Microsoft => "Microsoft",
            ProviderKind::GitHub => "GitHub",
            ProviderKind::GitLab => "GitLab",
            ProviderKind::Facebook => "Facebook",
            ProviderKind::Discord => "Discord",
            ProviderKind::Oidc => "Single sign-on",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OAuthProvider {
    pub kind: ProviderKind,
    pub display_name: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub scopes: String,
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl OAuthProvider {
    pub fn from_env(kind: ProviderKind) -> Option<Self> {
        let p = kind.env_prefix();
        let client_id = env_var(&format!("{p}_CLIENT_ID"))?;
        let client_secret = env_var(&format!("{p}_CLIENT_SECRET"))?;
        let display_name =
            env_var(&format!("{p}_DISPLAY_NAME")).unwrap_or_else(|| kind.default_name().into());

        let (auth_url, token_url, userinfo_url, scopes) = match kind {
            ProviderKind::Google => (
                "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                "https://oauth2.googleapis.com/token".to_string(),
                "https://openidconnect.googleapis.com/v1/userinfo".to_string(),
                "openid email profile",
            ),
            ProviderKind::Microsoft => {
                let tenant = env_var("MICROSOFT_TENANT").unwrap_or_else(|| "common".into());
                (
                    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
                    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                    "https://graph.microsoft.com/oidc/userinfo".to_string(),
                    "openid email profile",
                )
            }
            ProviderKind::GitHub => (
                "https://github.com/login/oauth/authorize".to_string(),
                "https://github.com/login/oauth/access_token".to_string(),
                "https://api.github.com/user".to_string(),
                "read:user user:email",
            ),
            ProviderKind::GitLab => {
                let base = env_var("GITLAB_BASE_URL")
                    .unwrap_or_else(|| "https://gitlab.com".into())
                    .trim_end_matches('/')
                    .to_string();
                (
                    format!("{base}/oauth/authorize"),
                    format!("{base}/oauth/token"),
                    format!("{base}/oauth/userinfo"),
                    "openid email profile",
                )
            }
            ProviderKind::Facebook => (
                "https://www.facebook.com/v19.0/dialog/oauth".to_string(),
                "https://graph.facebook.com/v19.0/oauth/access_token".to_string(),
                "https://graph.facebook.com/me?fields=id,name,email".to_string(),
                "email public_profile",
            ),
            ProviderKind::Discord => (
                "https://discord.com/oauth2/authorize".to_string(),
                "https://discord.com/api/oauth2/token".to_string(),
                "https://discord.com/api/users/@me".to_string(),
                "identify email",
            ),
            ProviderKind::Oidc => (
                env_var("OIDC_AUTH_URL")?,
                env_var("OIDC_TOKEN_URL")?,
                env_var("OIDC_USERINFO_URL")?,
                "openid email profile",
            ),
        };
        let scopes = env_var(&format!("{p}_SCOPES")).unwrap_or_else(|| scopes.to_string());
        Some(OAuthProvider {
            kind,
            display_name,
            client_id,
            client_secret,
            auth_url,
            token_url,
            userinfo_url,
            scopes,
        })
    }

    pub fn slug(&self) -> &'static str {
        self.kind.slug()
    }

    pub fn redirect_uri(&self, base_url: &str) -> String {
        format!("{base_url}/auth/{}/callback", self.slug())
    }

    /// URL to send the browser to.
    pub fn authorize_url(&self, base_url: &str, state: &str, pkce_verifier: &str) -> String {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce_verifier.as_bytes()));
        let mut url = url::Url::parse(&self.auth_url).expect("valid provider auth URL");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri(base_url))
            .append_pair("scope", &self.scopes)
            .append_pair("state", state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if self.kind == ProviderKind::Google {
            url.query_pairs_mut().append_pair("prompt", "select_account");
        }
        url.to_string()
    }

    /// Exchange the authorization code and fetch the user's profile.
    pub async fn fetch_profile(
        &self,
        http: &reqwest::Client,
        base_url: &str,
        code: &str,
        pkce_verifier: &str,
    ) -> Result<ExternalProfile, AppError> {
        let redirect_uri = self.redirect_uri(base_url);
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("code_verifier", pkce_verifier),
        ];
        let token: Value = http
            .post(&self.token_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await
            .map_err(oauth_err)?
            .json()
            .await
            .map_err(oauth_err)?;
        let access_token = token
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                tracing::warn!("{} token response without access_token: {token}", self.slug());
                AppError::BadRequest("The sign-in provider did not return an access token.".into())
            })?
            .to_string();

        let info: Value = self.get_json(http, &self.userinfo_url, &access_token).await?;

        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let mut profile = match self.kind {
            ProviderKind::GitHub => ExternalProfile {
                subject: info.get("id").map(|v| v.to_string()).unwrap_or_default(),
                email: s(&info, "email"),
                email_verified: false,
                name: s(&info, "name").or_else(|| s(&info, "login")),
            },
            ProviderKind::Facebook => ExternalProfile {
                subject: s(&info, "id").unwrap_or_default(),
                email: s(&info, "email"),
                email_verified: true,
                name: s(&info, "name"),
            },
            ProviderKind::Discord => ExternalProfile {
                subject: s(&info, "id").unwrap_or_default(),
                email: s(&info, "email"),
                email_verified: info.get("verified").and_then(Value::as_bool).unwrap_or(false),
                name: s(&info, "global_name").or_else(|| s(&info, "username")),
            },
            // OpenID Connect userinfo (Google, Microsoft, GitLab, generic OIDC)
            _ => ExternalProfile {
                subject: s(&info, "sub").unwrap_or_default(),
                email: s(&info, "email").or_else(|| s(&info, "preferred_username")),
                email_verified: info
                    .get("email_verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                name: s(&info, "name"),
            },
        };

        // GitHub hides private emails from /user; ask /user/emails.
        if self.kind == ProviderKind::GitHub {
            if let Ok(emails) = self
                .get_json(http, "https://api.github.com/user/emails", &access_token)
                .await
            {
                if let Some(primary) = emails.as_array().and_then(|list| {
                    list.iter().find(|e| {
                        e.get("primary").and_then(Value::as_bool) == Some(true)
                            && e.get("verified").and_then(Value::as_bool) == Some(true)
                    })
                }) {
                    profile.email = s(primary, "email");
                    profile.email_verified = true;
                }
            }
        }

        if profile.subject.is_empty() || profile.subject == "null" {
            return Err(AppError::BadRequest(
                "The sign-in provider did not identify the account.".into(),
            ));
        }
        Ok(profile)
    }

    async fn get_json(
        &self,
        http: &reqwest::Client,
        url: &str,
        access_token: &str,
    ) -> Result<Value, AppError> {
        let resp = http
            .get(url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("User-Agent", "greenhouse-tracker")
            .send()
            .await
            .map_err(oauth_err)?;
        if !resp.status().is_success() {
            return Err(AppError::BadRequest(format!(
                "The sign-in provider returned {} when fetching your profile.",
                resp.status()
            )));
        }
        resp.json().await.map_err(oauth_err)
    }
}

fn oauth_err(e: reqwest::Error) -> AppError {
    tracing::warn!("oauth request failed: {e}");
    AppError::BadRequest("Could not contact the sign-in provider. Please try again.".into())
}

/// The identity the provider vouched for.
#[derive(Debug, Clone)]
pub struct ExternalProfile {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
}
