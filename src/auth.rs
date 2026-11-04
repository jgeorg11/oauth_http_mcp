use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::Jwk};
use jwks_cache::{IdentityProviderRegistration, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, RwLock},
    time::Instant,
};

use crate::config::AuthConfig;

#[derive(Clone)]
pub struct AuthState {
    pub(crate) config: AuthConfig,
    registry: Arc<RwLock<Registry>>,
    registration: IdentityProviderRegistration,
    unknown_key_refresh: Arc<Mutex<Option<Instant>>>,
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
    oid: String,
    ver: String,
    #[serde(default)]
    scp: String,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub tenant_id: String,
    pub object_id: String,
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

        let registry = Registry::new();
        let registration = Self::key_registration(&config, &discovery.jwks_uri)?;
        registry.register(registration.clone()).await?;
        registry.resolve(&config.tenant_id, "entra", None).await?;
        Ok(Self::from_registry(config, registry, registration))
    }

    fn key_registration(
        config: &AuthConfig,
        jwks_uri: &str,
    ) -> Result<IdentityProviderRegistration, jwks_cache::Error> {
        let mut registration =
            IdentityProviderRegistration::new(&config.tenant_id, "entra", jwks_uri)?;
        registration.max_ttl = Duration::from_secs(60 * 60);
        // Stay below the default 30-second minimum TTL, including jitter.
        registration.refresh_early = Duration::from_secs(10);
        Ok(registration)
    }

    fn from_registry(
        config: AuthConfig,
        registry: Registry,
        registration: IdentityProviderRegistration,
    ) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&config.audience]);
        validation.set_issuer(&[config.issuer()]);
        validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        Self {
            config,
            registry: Arc::new(RwLock::new(registry)),
            registration,
            unknown_key_refresh: Arc::new(Mutex::new(None)),
            validation,
        }
    }

    pub async fn refresh_keys_periodically(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            // resolve starts a background refresh only when the cache is due.
            // Polling also refreshes keys while no protected requests arrive.
            let registry = self.registry.read().await.clone();
            if let Err(error) = registry
                .resolve(&self.config.tenant_id, "entra", None)
                .await
            {
                tracing::warn!(%error, "Periodic JWKS cache check failed");
            }
        }
    }

    async fn refresh_keys_for_unknown_kid(&self, kid: &str) -> Result<(), jwks_cache::Error> {
        // One lock and timestamp per provider, shared even by AuthState clones.
        // Waiters recheck the cache after the in-flight refresh completes.
        let mut last_attempt = self.unknown_key_refresh.lock().await;
        let registry = self.registry.read().await.clone();
        let keys = registry
            .resolve(&self.config.tenant_id, "entra", Some(kid))
            .await?;
        if keys.find(kid).is_some() {
            return Ok(());
        }
        if last_attempt.is_some_and(|last| last.elapsed() < Duration::from_mins(5)) {
            return Ok(());
        }
        // Failed or cancelled refreshes also consume the cooldown.
        *last_attempt = Some(Instant::now());

        // jwks-cache 0.2's refresh() only refreshes entries that are already due.
        // Load a replacement first so a failed fetch preserves the active cache.
        let replacement = Registry::builder()
            .require_https(self.registration.require_https)
            .build();
        replacement.register(self.registration.clone()).await?;
        replacement
            .resolve(&self.config.tenant_id, "entra", None)
            .await?;
        *self.registry.write().await = replacement;
        Ok(())
    }

    async fn verify_token_with_cache(&self, token: &str) -> Result<Claims, StatusCode> {
        let header = decode_header(token).map_err(|_| StatusCode::UNAUTHORIZED)?;
        if header.alg != Algorithm::RS256 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let kid = header.kid.ok_or(StatusCode::UNAUTHORIZED)?;
        let registry = self.registry.read().await.clone();
        let mut keys = registry
            .resolve(&self.config.tenant_id, "entra", Some(&kid))
            .await
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        if keys.find(&kid).is_none() {
            self.refresh_keys_for_unknown_kid(&kid)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Unknown signing key JWKS refresh failed");
                    StatusCode::UNAUTHORIZED
                })?;
            let registry = self.registry.read().await.clone();
            keys = registry
                .resolve(&self.config.tenant_id, "entra", Some(&kid))
                .await
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
        }
        let jwk = keys.find(&kid).ok_or(StatusCode::UNAUTHORIZED)?;
        // jwks-cache 0.2 uses jsonwebtoken 10; this application uses version 11.
        // Convert the selected public JWK between their distinct Rust types.
        let jwk: Jwk = serde_json::from_value(
            serde_json::to_value(jwk).map_err(|_| StatusCode::UNAUTHORIZED)?,
        )
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(decode::<Claims>(token, &key, &self.validation)
            .map_err(|_| StatusCode::UNAUTHORIZED)?
            .claims)
    }

    async fn validate(&self, token: &str) -> Result<Principal, StatusCode> {
        let claims = self.verify_token_with_cache(token).await?;

        if claims.tid != self.config.tenant_id
            || claims.ver != "2.0"
            || claims.oid.trim().is_empty()
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        if !claims
            .scp
            .split_ascii_whitespace()
            .any(|s| s == "access_as_user")
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(Principal {
            tenant_id: claims.tid,
            object_id: claims.oid,
        })
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
    mut request: Request,
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
    match auth.validate(token).await {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
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
