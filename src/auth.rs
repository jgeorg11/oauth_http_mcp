use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::AuthConfig;

pub struct AuthState {
    pub(crate) config: AuthConfig,
    keys: JwkSet,
    validation: Validation,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct Claims {
    tid: String,
    ver: String,
    #[serde(default)]
    scp: String,
}

impl AuthState {
    pub async fn new(config: AuthConfig) -> Result<Self, Box<dyn Error>> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        let discovery: Discovery = client
            .get(format!(
                "{}/.well-known/openid-configuration",
                config.issuer()
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if discovery.issuer != config.issuer() {
            return Err("Entra discovery returned an unexpected issuer".into());
        }

        // Demo: load keys once. Restart to refresh them after a signing-key rotation.
        let keys = client
            .get(discovery.jwks_uri)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Self::from_keys(config, keys))
    }

    fn from_keys(config: AuthConfig, keys: JwkSet) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&config.audience]);
        validation.set_issuer(&[config.issuer()]);
        validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        Self {
            config,
            keys,
            validation,
        }
    }

    fn validate(&self, token: &str) -> Result<(), StatusCode> {
        let invalid = StatusCode::UNAUTHORIZED;
        let header = decode_header(token).map_err(|_| invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(invalid);
        }
        let kid = header.kid.ok_or(invalid)?;
        let jwk = self.keys.find(&kid).ok_or(invalid)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| invalid)?;
        let claims = decode::<Claims>(token, &key, &self.validation)
            .map_err(|_| invalid)?
            .claims;

        if claims.tid != self.config.tenant_id || claims.ver != "2.0" {
            return Err(invalid);
        }
        if !claims
            .scp
            .split_ascii_whitespace()
            .any(|s| s == "access_as_user")
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(())
    }

    fn reject(&self, status: StatusCode, error: Option<&str>) -> Response {
        let mut challenge = format!(
            "Bearer resource_metadata=\"{}\", scope=\"{}\"",
            self.config.metadata_url(),
            self.config.scope(),
        );
        if let Some(error) = error {
            challenge.push_str(&format!(", error=\"{error}\""));
        }
        (status, [(header::WWW_AUTHENTICATE, challenge)]).into_response()
    }
}

pub async fn require_auth(
    State(auth): State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token);

    let Some(token) = token else {
        return auth.reject(StatusCode::UNAUTHORIZED, None);
    };
    match auth.validate(token) {
        Ok(()) => next.run(request).await,
        Err(StatusCode::FORBIDDEN) => {
            auth.reject(StatusCode::FORBIDDEN, Some("insufficient_scope"))
        }
        Err(_) => auth.reject(StatusCode::UNAUTHORIZED, Some("invalid_token")),
    }
}

pub async fn metadata(State(auth): State<Arc<AuthState>>) -> Json<Value> {
    Json(json!({
        "resource": auth.config.resource.as_str(),
        "authorization_servers": [auth.config.issuer()],
        "scopes_supported": [auth.config.scope()],
        "bearer_methods_supported": ["header"],
    }))
}
